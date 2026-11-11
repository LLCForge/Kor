// Sayfa araçları: bağlam ekle ("bu sayfayı Kor'a ver", §18.3), dokümantasyon okuyucu (§27.11), form doldurucu
// (§27.12), sayfa → kod (§27.13). Sayfayı eklenti okur; anlamlandırma ve üretim çekirdekte.
import { useEffect, useState } from 'react'
import Markdown from 'react-markdown'
import { call, client } from './bus.ts'
import { DiffView, type FileChange } from './DiffView.tsx'
import type { Context } from './Chat.tsx'
import { t } from './i18n.ts'

interface PageText {
  url: string
  title: string
  text: string
  selection: string
}

export function ContextTools({ setContext, onAdopt }: { setContext: (c: Context) => void; onAdopt: (msg: string) => void }) {
  const [error, setError] = useState('')
  const page = (what: 'page' | 'selection') =>
    void call<PageText>('pageText').then(
      (p) => setContext(what === 'page' ? { url: p.url, title: p.title, text: p.text } : { url: p.url, title: p.title, selection: p.selection }),
      (e: Error) => setError(e.message),
    )
  const shot = () =>
    void call<{ dataUrl: string; url: string; title: string }>('screenshot').then((s) => setContext({ url: s.url, title: s.title, screenshot: s.dataUrl }), (e: Error) => setError(e.message))
  const adopt = () =>
    void call<{ title: string }>('adopt').then((g) => onAdopt(`${t('Sekme Kor grubuna verildi')}: ${g.title}`), (e: Error) => setError(e.message))
  return (
    <div className="tools">
      <h2>{t('Bu sayfayı Kor\'a ver')}</h2>
      <div className="cmds">
        <button type="button" onClick={() => page('page')}>{t('Sayfa metni')}</button>
        <button type="button" onClick={() => page('selection')}>{t('Seçili metin')}</button>
        <button type="button" onClick={shot}>{t('Ekran görüntüsü')}</button>
        <button type="button" onClick={adopt} title={t('Kor yalnızca kendi grubundaki sekmelerde çalışır; bu sekmeyi göreve verir.')}>
          {t('Bu sekmeyi Kor\'a ver')}
        </button>
      </div>
      {error ? <p className="err">{error}</p> : null}
    </div>
  )
}

interface DocsAnswer {
  library: { name: string; page_version: string; project_version: string; mismatch: boolean } | null
  answer: string
  places: string[]
  cached: boolean
}

export function DocsReader({ pending, onApplied }: { pending: { selection: string; url: string; title: string } | null; onApplied: (sessionId: string) => void }) {
  const [question, setQuestion] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [page, setPage] = useState<PageText | null>(null)
  const [answer, setAnswer] = useState<DocsAnswer | null>(null)

  const run = (selection?: { selection: string; url: string; title: string }) => {
    setBusy(true)
    setError('')
    setAnswer(null)
    const read = selection ? call<PageText>('pageText').then((p) => ({ ...p, selection: selection.selection })) : call<PageText>('pageText')
    read
      .then((p) => {
        setPage(p)
        return client<DocsAnswer>('docs', { page: p, question }, 600000)
      })
      .then(setAnswer, (e: Error) => setError(e.message))
      .finally(() => setBusy(false))
  }
  useEffect(() => {
    if (pending) run(pending)
  }, [pending])

  const apply = () =>
    page &&
    answer &&
    void client<{ sessionId: string }>('docsApply', { page, question, answer: answer.answer }).then((r) => onApplied(r.sessionId), (e: Error) => setError(e.message))

  return (
    <div className="tools">
      <h2>{t('Dokümantasyon okuyucu')}</h2>
      <div className="row">
        <input type="text" aria-label={t('Soru')} placeholder={t('Bunu projemde nasıl kullanırım?')} value={question} onChange={(e) => setQuestion(e.target.value)} />
      </div>
      <button type="button" className="primary" disabled={busy} onClick={() => run()} data-kor-cmd="client.docs">
        {busy ? t('Okunuyor…') : t('Bu sayfayı projemde kullan')}
      </button>
      {error ? <p className="err">{error}</p> : null}
      {answer ? (
        <div className="answer">
          {answer.library ? (
            <p className={answer.library.mismatch ? 'warn' : 'note'} translate="no">
              {answer.library.name}: {t('sayfa')} {answer.library.page_version || '—'}, {t('proje')} {answer.library.project_version || '—'}
              {answer.library.mismatch ? ` — ${t('sürüm uyuşmazlığı: kod projenin sürümüne göre yazıldı')}` : ''}
            </p>
          ) : null}
          <div className="md" translate="no">
            <Markdown>{answer.answer}</Markdown>
          </div>
          {answer.places.length > 0 ? <p className="note" translate="no">{t('İlgili yerler')}: {answer.places.join(', ')}</p> : null}
          <button type="button" onClick={apply} data-kor-cmd="client.docsApply">
            {t('Uygula (dosya başına onayla)')}
          </button>
        </div>
      ) : null}
    </div>
  )
}

interface Field {
  ref: string
  label: string
  name: string
  id: string
  placeholder: string
  autocomplete: string
  kind: string
  options: string[]
  sensitive: boolean
}
interface Fill {
  ref: string
  key: string
  value: string
  label: string
  how: string
}

export function FormFiller() {
  const [profiles, setProfiles] = useState<string[]>([])
  const [profile, setProfile] = useState('')
  const [drafts, setDrafts] = useState(false)
  const [hint, setHint] = useState('')
  const [fields, setFields] = useState<Field[]>([])
  const [fills, setFills] = useState<Fill[]>([])
  const [refused, setRefused] = useState<string[]>([])
  const [unmatched, setUnmatched] = useState<string[]>([])
  const [result, setResult] = useState('')
  const [corrections, setCorrections] = useState<Record<string, string> | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    client<string[]>('formProfiles').then((p) => {
      setProfiles(p)
      setProfile(p[0] ?? '')
    }, () => setProfiles([]))
  }, [])

  const read = () => {
    setError('')
    setResult('')
    setCorrections(null)
    setBusy(true)
    let sensitiveLabels: string[] = []
    call<{ fields: Field[]; title: string; url: string }>('formRead')
      .then((r) => {
        setFields(r.fields)
        sensitiveLabels = r.fields.filter((f) => f.sensitive).map((f) => f.label || f.name)
        return client<{ fills: Fill[]; refused: string[]; unmatched: string[] }>('formPlan', {
          fields: r.fields.filter((f) => !f.sensitive),
          profile,
          drafts,
          hint,
          page: { title: r.title, url: r.url },
        })
      })
      .then((plan) => {
        setFills(plan.fills)
        setRefused([...new Set([...plan.refused, ...sensitiveLabels])])
        setUnmatched(plan.unmatched)
      }, (e: Error) => setError(e.message))
      .finally(() => setBusy(false))
  }
  const fill = () =>
    void call<{ filled: string[]; refused: string[]; missing: string[] }>('formFill', { fills: fills.map((f) => ({ ref: f.ref, value: f.value })) }).then(
      (r) => setResult(`${t('Dolduruldu')}: ${r.filled.length} · ${t('yazılmadı')}: ${r.refused.length + r.missing.length}. ${t('Alanları kontrol edip formu siz gönderin.')}`),
      (e: Error) => setError(e.message),
    )
  const learn = () =>
    void call<{ values: Record<string, string> }>('formValues', { refs: fills.map((f) => f.ref) })
      .then((r) => client<Record<string, string>>('formLearn', { filled: fills, now: r.values }))
      .then(setCorrections, (e: Error) => setError(e.message))
  const confirmLearn = () =>
    corrections &&
    void client('formLearnConfirm', { profile, corrections }).then(() => {
      setCorrections(null)
      setResult(t('Profil güncellendi.'))
    }, (e: Error) => setError(e.message))

  return (
    <div className="tools">
      <h2>{t('Form doldurucu')}</h2>
      {profiles.length === 0 ? <p className="note">{t('Profil yok: Kor masaüstü > Ayarlar > Tarayıcı eklentisi ya da kor chrome profile set')}</p> : null}
      <div className="row">
        <select aria-label={t('Profil')} value={profile} onChange={(e) => setProfile(e.target.value)}>
          {profiles.map((p) => (
            <option key={p} value={p} translate="no">
              {p}
            </option>
          ))}
        </select>
        <label className="sw">
          <input type="checkbox" checked={drafts} onChange={(e) => setDrafts(e.target.checked)} />
          {t('Serbest metin taslakları')}
        </label>
      </div>
      {drafts ? <input type="text" aria-label={t('Taslak için ipucu')} placeholder={t('Taslak için ipucu (ör. deneyimim)')} value={hint} onChange={(e) => setHint(e.target.value)} /> : null}
      <button type="button" className="primary" disabled={busy} onClick={read} data-kor-cmd="client.formPlan">
        {busy ? t('Okunuyor…') : t('Formu oku ve eşleştir')}
      </button>
      {fills.length > 0 ? (
        <table className="ftable">
          <thead>
            <tr>
              <th>{t('Alan')}</th>
              <th>{t('Değer')}</th>
            </tr>
          </thead>
          <tbody>
            {fills.map((f, i) => (
              <tr key={f.ref}>
                <td translate="no">
                  {f.label} <span className="note">({f.how === 'draft' ? t('taslak') : f.key})</span>
                </td>
                <td>
                  <input
                    type="text"
                    aria-label={f.label}
                    value={f.value}
                    onChange={(e) => setFills(fills.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : fields.length > 0 && !busy ? (
        <p className="note">{t('Eşleşen alan yok.')}</p>
      ) : null}
      {refused.length > 0 ? <p className="warn">{t('Kor bu alanlara yazmaz (parola, kart, kod)')}: {refused.join(', ')}</p> : null}
      {unmatched.length > 0 ? <p className="note">{t('Eşleşmeyen')}: {unmatched.join(', ')}</p> : null}
      {fills.length > 0 ? (
        <div className="row">
          <button type="button" onClick={fill}>{t('Doldur (göndermez)')}</button>
          <button type="button" onClick={learn}>{t('Düzeltmelerimi öğren')}</button>
        </div>
      ) : null}
      {corrections ? (
        Object.keys(corrections).length === 0 ? (
          <p className="note">{t('Değiştirdiğiniz değer yok.')}</p>
        ) : (
          <div className="confirm">
            <div className="q">{t('Profile kaydedilsin mi?')}</div>
            <ul translate="no">
              {Object.entries(corrections).map(([k, v]) => (
                <li key={k}>
                  {k}: {v}
                </li>
              ))}
            </ul>
            <button type="button" className="primary" onClick={confirmLearn} data-kor-cmd="client.formLearnConfirm">
              {t('Kaydet')}
            </button>
          </div>
        )
      ) : null}
      {result ? <p className="note" role="status">{result}</p> : null}
      {error ? <p className="err">{error}</p> : null}
    </div>
  )
}

interface Capture {
  screenshot: string
  removed: string[]
  width: number
  height: number
  url: string
  [k: string]: unknown
}
interface Generated {
  target: string
  similarity: number | null
  notes: string[]
  vision: boolean
  files: FileChange[]
}

export function PageToCode() {
  const [target, setTarget] = useState('')
  const [story, setStory] = useState(false)
  const [test, setTest] = useState(false)
  const [cap, setCap] = useState<Capture | null>(null)
  const [gen, setGen] = useState<Generated | null>(null)
  const [keep, setKeep] = useState<Record<string, boolean>>({})
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [written, setWritten] = useState<string[]>([])

  const pick = () => {
    setError('')
    setGen(null)
    setWritten([])
    setBusy(t('Sayfada bir bileşene tıklayın (Esc: iptal)'))
    call<Capture>('pick').then(setCap, (e: Error) => setError(e.message)).finally(() => setBusy(''))
  }
  const generate = () => {
    if (!cap) return
    setBusy(t('Kod üretiliyor…'))
    setError('')
    client<Generated>('pageCode', { capture: cap, options: { target, story, test } }, 900000)
      .then((g) => {
        setGen(g)
        setKeep(Object.fromEntries(g.files.map((f) => [f.path, true])))
      }, (e: Error) => setError(e.message))
      .finally(() => setBusy(''))
  }
  const write = () => {
    if (!gen) return
    const files = gen.files.filter((f) => keep[f.path]).map((f) => ({ path: f.path, content: f.after ?? '' }))
    client<{ written: string[] }>('pageCodeWrite', { files }).then((r) => setWritten(r.written), (e: Error) => setError(e.message))
  }

  return (
    <div className="tools">
      <h2>{t('Sayfa → kod')}</h2>
      <div className="row">
        <select aria-label={t('Hedef')} value={target} onChange={(e) => setTarget(e.target.value)}>
          <option value="">{t('Projenin yığını')}</option>
          {['react', 'vue', 'svelte', 'solid', 'angular', 'leptos', 'html'].map((x) => (
            <option key={x} value={x}>
              {x}
            </option>
          ))}
        </select>
        <label className="sw">
          <input type="checkbox" checked={story} onChange={(e) => setStory(e.target.checked)} />
          Storybook
        </label>
        <label className="sw">
          <input type="checkbox" checked={test} onChange={(e) => setTest(e.target.checked)} />
          {t('Test')}
        </label>
      </div>
      <button type="button" className="primary" disabled={!!busy} onClick={pick}>
        {t('Bileşen seç')}
      </button>
      {busy ? <p className="note" role="status">{busy}</p> : null}
      {cap ? (
        <div>
          <img className="shot" src={cap.screenshot} alt={t('Seçilen bileşen')} />
          {cap.removed.length > 0 ? <p className="note">{t('Görseller ve logolar kopyalanmaz, yer tutucuyla değiştirildi')}: {cap.removed.length}</p> : null}
          <button type="button" disabled={!!busy} onClick={generate} data-kor-cmd="client.pageCode">
            {t('Bunu projemde yap')}
          </button>
        </div>
      ) : null}
      {gen ? (
        <div>
          <p>
            {gen.target} · {gen.similarity === null ? t('görsel karşılaştırma yok') : `${t('orijinale yakınlık')} %${gen.similarity}`}
          </p>
          {gen.notes.map((n, i) => (
            <p key={i} className="note">{n}</p>
          ))}
          {gen.files.map((f) => (
            <div key={f.path} className="file-review">
              <DiffView file={f} />
              <label className="sw">
                <input type="checkbox" checked={keep[f.path] ?? false} onChange={(e) => setKeep({ ...keep, [f.path]: e.target.checked })} />
                {t('Bu dosyayı yaz')}
              </label>
            </div>
          ))}
          <button type="button" className="primary" onClick={write} data-kor-cmd="client.pageCodeWrite">
            {t('Seçilen dosyaları yaz')}
          </button>
          {written.length > 0 ? <p className="note" translate="no">{t('Yazıldı')}: {written.join(', ')}</p> : null}
        </div>
      ) : null}
      {error ? <p className="err">{error}</p> : null}
    </div>
  )
}
