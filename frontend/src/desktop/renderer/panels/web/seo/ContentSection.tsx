// İçerik hattı (§29.7): brief → taslak plan → metin → SEO kontrolü → insan incelemesi → yayın. Her aşama
// düzenlenebilir bir Markdown dosyasıdır; onay taslağın özetine bağlıdır, onaydan sonra değişen taslak
// yayımlanmaz. Yayın: WordPress'e taslak (ya da onayla yayımla) veya dosyaya dışa aktarma.
import { Check, FilePlus2, PenLine, Play, RefreshCw, Save, Send } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { SeoContentFiles } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { confirmAction } from '../../../ConfirmDialog.tsx'
import { seoApi } from './api.ts'
import { parseUrls } from './format.ts'

const STAGE_LABEL: Record<string, string> = {
  new: 'yeni',
  brief: 'brief hazır',
  outline: 'plan hazır',
  draft: 'taslak hazır',
  checked: 'kontrol edildi',
  approved: 'onaylandı',
  published: 'yayımlandı',
}

type FileName = 'brief.md' | 'outline.md' | 'draft.md'

export function ContentSection() {
  const uid = useId()
  const list = useSWR('seo.contentList', () => seoApi.contentList())
  const [keyword, setKeyword] = useState('')
  const [fromUrl, setFromUrl] = useState('')
  const [open, setOpen] = useState<SeoContentFiles | null>(null)
  const [file, setFile] = useState<FileName>('brief.md')
  const [text, setText] = useState('')
  const [serp, setSerp] = useState('')
  const [site, setSite] = useState('')
  const [wpSite, setWpSite] = useState('')
  const [out, setOut] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)

  const load = async (slug: string, f: FileName = file) => {
    const x = await seoApi.contentRead(slug)
    setOpen(x)
    setFile(f)
    setText(x[f.replace('.md', '') as 'brief' | 'outline' | 'draft'])
  }
  const act = (f: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    setNote(null)
    f().catch((e: unknown) => setError(errorMessage(e))).finally(() => { setBusy(false); void list.mutate() })
  }
  const create = (e: FormEvent) => {
    e.preventDefault()
    act(async () => {
      const st = await seoApi.contentNew(keyword.trim(), null, fromUrl.trim() || null)
      setKeyword('')
      setFromUrl('')
      await load(st.slug, 'brief.md')
    })
  }
  const run = (stage: string) => act(async () => {
    if (!open) return
    await seoApi.contentRun(open.state.slug, stage, parseUrls(serp), site.trim() || null, null)
    const next: FileName = stage === 'brief' ? 'brief.md' : stage === 'outline' ? 'outline.md' : 'draft.md'
    await load(open.state.slug, next)
  })
  const save = () => act(async () => {
    if (!open) return
    await seoApi.contentSave(open.state.slug, file, text)
    await load(open.state.slug, file)
    setNote('Kaydedildi. Taslak değiştiyse yeniden kontrol edip onaylayın.')
  })
  const approve = () => act(async () => {
    if (!open) return
    await seoApi.contentApprove(open.state.slug)
    await load(open.state.slug, file)
  })
  const publish = (live: boolean) => act(async () => {
    if (!open) return
    const toWp = wpSite.trim() !== ''
    const ok = await confirmAction({
      title: toWp ? (live ? 'WordPress\'te yayımla' : 'WordPress\'e taslak gönder') : 'Dışa aktar',
      message: toWp ? `"${open.state.keyword}" ${wpSite} sitesine ${live ? 'yayımlanmış yazı' : 'taslak'} olarak gönderilecek.` : `"${open.state.keyword}" ${out} dosyasına yazılacak.`,
      confirmLabel: live ? 'Yayımla' : 'Gönder',
      danger: live,
    })
    if (!ok) return
    const st = await seoApi.contentPublish(open.state.slug, toWp ? wpSite.trim() : null, live, toWp ? null : out.trim())
    setNote(`Tamam: ${JSON.stringify(st.published)}`)
    await load(open.state.slug, file)
  })

  const stage = open?.state.stage
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`} data-kor-event="kor:seo/progress">
      <div className="kg-card-head">
        <h3 id={`${uid}-h`} className="kg-card-title ks-title">
          <PenLine size={16} aria-hidden="true" />
          İçerik hattı
        </h3>
        <button type="button" className="action-btn" onClick={() => void list.mutate()} aria-label="İçerik listesini yenile" title="Yenile" data-kor-cmd="seo.contentList">
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      <form className="kp-form" onSubmit={create}>
        <label className="kg-field kp-grow">
          <span>Anahtar kelime</span>
          <input value={keyword} onChange={(e) => setKeyword(e.target.value)} placeholder="yurtdışı kargo" maxLength={200} required />
        </label>
        <label className="kg-field kp-grow">
          <span>Mevcut yazıyı güncelle (URL, isteğe bağlı)</span>
          <input value={fromUrl} onChange={(e) => setFromUrl(e.target.value)} placeholder="https://ornek.com/eski-yazi" spellCheck={false} maxLength={500} translate="no" />
        </label>
        <button type="submit" className="btn small" disabled={busy || !keyword.trim()} data-kor-cmd="seo.contentNew">
          <FilePlus2 size={14} aria-hidden="true" />
          Başlat
        </button>
      </form>
      {list.error ? (
        <p className="error-text" role="alert" translate="no">{errorMessage(list.error)}</p>
      ) : !list.data ? (
        <p className="muted" role="status">İçerikler okunuyor…</p>
      ) : list.data.length === 0 ? (
        <p className="ks-empty">Henüz içerik yok. Dosyalar <code>.kor/seo/content/</code> altına yazılır; marka sesi için <code>.kor/brand-voice.md</code>.</p>
      ) : (
        <ul className="ks-list">
          {list.data.map((c) => (
            <li key={c.slug}>
              <button type="button" className="kg-link" aria-current={open?.state.slug === c.slug} onClick={() => act(() => load(c.slug))} data-kor-cmd="seo.contentRead">
                <span translate="no">{c.keyword}</span>
                <span className="kg-link-meta">{STAGE_LABEL[c.stage] ?? c.stage}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {open && (
        <div className="ks-form">
          <p className="kp-small">{`${open.state.keyword} · ${STAGE_LABEL[open.state.stage] ?? open.state.stage}${open.state.model ? ` · ${open.state.model}` : ''}`}</p>
          <label className="ks-field">
            <span>Şu an sıralanan sayfalar (brief için, satır başına bir URL; Kor robots.txt'e uyarak okur)</span>
            <textarea className="kseo-text kseo-short" value={serp} onChange={(e) => setSerp(e.target.value)} spellCheck={false} translate="no" />
          </label>
          <label className="kg-field">
            <span>İç link önerileri için site (son denetimden)</span>
            <input value={site} onChange={(e) => setSite(e.target.value)} placeholder="ornek.com" spellCheck={false} translate="no" />
          </label>
          <div className="kseo-tabs">
            {(['brief', 'outline', 'draft'] as const).map((s) => (
              <button key={s} type="button" className="btn small" disabled={busy} onClick={() => run(s)} data-kor-cmd="seo.contentRun">
                <Play size={14} aria-hidden="true" />
                {s === 'brief' ? 'Brief yaz' : s === 'outline' ? 'Plan yaz' : 'Metni yaz'}
              </button>
            ))}
            <button type="button" className="btn small" disabled={busy || !open.draft.trim()} onClick={() => run('check')} data-kor-cmd="seo.contentRun">SEO kontrolü</button>
            <button type="button" className="btn small" disabled={busy || stage !== 'checked'} onClick={approve} data-kor-cmd="seo.contentApprove">
              <Check size={14} aria-hidden="true" />
              Onayla
            </button>
          </div>
          <div className="kseo-tabs" role="tablist" aria-label="Dosya">
            {(['brief.md', 'outline.md', 'draft.md'] as const).map((f) => (
              <button key={f} type="button" role="tab" aria-selected={file === f} className={file === f ? 'btn small' : 'btn small ghost'} onClick={() => act(() => load(open.state.slug, f))} data-kor-cmd="seo.contentRead">{f}</button>
            ))}
          </div>
          <textarea className="kseo-text" value={text} onChange={(e) => setText(e.target.value)} aria-label={file} translate="no" />
          <button type="button" className="btn small" disabled={busy} onClick={save} data-kor-cmd="seo.contentSave">
            <Save size={14} aria-hidden="true" />
            Kaydet
          </button>
          {open.check && <pre className="kp-pre kp-small" translate="no">{open.check}</pre>}
          <div className="kp-form">
            <label className="kg-field">
              <span>WordPress sitesi (kor wp connect)</span>
              <input value={wpSite} onChange={(e) => setWpSite(e.target.value)} placeholder="site-adi" spellCheck={false} translate="no" />
            </label>
            <label className="kg-field kp-grow">
              <span>ya da dışa aktarma dosyası</span>
              <input value={out} onChange={(e) => setOut(e.target.value)} placeholder="icerik/yazi.md" spellCheck={false} translate="no" />
            </label>
            <button type="button" className="btn small" disabled={busy || stage !== 'approved' || (!wpSite.trim() && !out.trim())} onClick={() => publish(false)} data-kor-cmd="seo.contentPublish">
              <Send size={14} aria-hidden="true" />
              {wpSite.trim() ? 'Taslak gönder' : 'Dışa aktar'}
            </button>
            {wpSite.trim() && (
              <button type="button" className="btn small danger" disabled={busy || stage !== 'approved'} onClick={() => publish(true)} data-kor-cmd="seo.contentPublish">Yayımla</button>
            )}
          </div>
        </div>
      )}
      {busy && <p className="muted kp-small" role="status">Çalışıyor…</p>}
      {note && <p className="muted kp-small" role="status" translate="no">{note}</p>}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
    </section>
  )
}
