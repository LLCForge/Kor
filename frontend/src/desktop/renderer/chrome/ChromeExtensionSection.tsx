// Ayarlar > Tarayıcı eklentisi (§18, §27.11–13, ADR 0058): kurulum (yerel mesajlaşma ana bilgisayarı, derleme
// klasörleri), eşleştirme kodu, tarayıcı araçlarının arka ucu, alan adı listeleri, form profilleri, dokümantasyon
// önbelleği. CLI karşılıkları: kor chrome install|pair|allow|deny|domains|profile, kor docs cache.
import { Copy, FolderOpen, Plus, Puzzle, RefreshCw, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from '../api.ts'

const INFO_KEY = 'chrome:info'

export function ChromeExtensionSection() {
  const info = useSWR(INFO_KEY, () => api.chromeExtension.info(), { revalidateOnFocus: true })
  const [code, setCode] = useState<{ code: string; expires: number } | null>(null)
  const [domain, setDomain] = useState('')
  const [message, setMessage] = useState<string | null>(null)
  const run = <T,>(p: Promise<T>, done?: (v: T) => void) =>
    void p.then(
      (v) => {
        done?.(v)
        void info.mutate()
      },
      (err: unknown) => setMessage(errorMessage(err)),
    )

  if (info.error) return <p className="error-text">{errorMessage(info.error)}</p>
  if (!info.data) return <p className="muted">Yükleniyor...</p>
  const d = info.data
  const hosts = Object.entries(d.hosts)

  return (
    <div className="g7-stack">
      <p className="g7-note text-pretty">
        <Puzzle size={12} aria-hidden="true" /> Kor eklentisi kendi tarayıcınızda (Chrome, Edge, Brave, Chromium, Firefox) çalışır: yan panel Kor'un üçüncü istemcisidir;
        tarayıcı araçları yalnızca Kor'un sekme grubunda, her işlemi onaylatarak çalışır. Parola ve kart alanlarına yazılmaz, formu siz gönderirsiniz.
      </p>

      <div className="g7-button-row">
        <span className="badge g9-on">{d.connected ? `eklenti bağlı (${d.connections})` : d.running ? 'eklenti bağlı değil' : 'köprü başka bir Kor sürecinde'}</span>
        <span className="g9-grow" />
        <button type="button" className="btn small" onClick={() => void info.mutate()} data-kor-cmd="chromeExtension.info">
          <RefreshCw size={14} aria-hidden="true" /> Yenile
        </button>
      </div>

      <section className="g7-subsection">
        <h4 className="g7-h4">1. Yerel ana bilgisayarı kaydet</h4>
        <p className="g9-small muted">Birincil bağlantı yolu ve yan panelden tek tıkla Kor'u başlatma için (yoksa eklenti yerel WebSocket'e düşer).</p>
        <ul className="g9-small">
          {hosts.map(([b, on]) => (
            <li key={b}>
              {b}: {on ? 'kayıtlı' : 'kayıtlı değil'}
            </li>
          ))}
        </ul>
        <div className="g7-button-row">
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.install(), (r) => setMessage(`Kaydedildi: ${r.installed.map((x) => x.browser).join(', ')}`))} data-kor-cmd="chromeExtension.install">
            Tüm tarayıcılar için kaydet
          </button>
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.uninstall(), (r) => setMessage(`Kaldırıldı: ${r.removed.join(', ') || '—'}`))} data-kor-cmd="chromeExtension.uninstall">
            Kayıtları kaldır
          </button>
        </div>
      </section>

      <section className="g7-subsection">
        <h4 className="g7-h4">2. Eklentiyi yükle</h4>
        <p className="g9-small muted">Chrome/Edge/Brave: Uzantılar &gt; Geliştirici modu &gt; "Paketlenmemiş öğe yükle" ile şu klasörü seçin:</p>
        <div className="g7-inline">
          <code className="g7-mono g7-path g7-grow" translate="no">{d.extensionPath || '—'}</code>
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.openFolder('chrome'))} data-kor-cmd="chromeExtension.openFolder">
            <FolderOpen size={14} aria-hidden="true" /> Klasörü aç
          </button>
        </div>
        <p className="g9-small muted">Firefox: about:debugging &gt; Bu Firefox &gt; "Geçici eklenti yükle" ile bu klasördeki manifest.json:</p>
        <div className="g7-inline">
          <code className="g7-mono g7-path g7-grow" translate="no">{d.firefoxPath || '—'}</code>
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.openFolder('firefox'))} data-kor-cmd="chromeExtension.openFolder">
            <FolderOpen size={14} aria-hidden="true" /> Klasörü aç
          </button>
        </div>
      </section>

      <section className="g7-subsection">
        <h4 className="g7-h4">3. Eşleştir</h4>
        <p className="g9-small muted">Tek kullanımlık kodu (10 dakika) yan paneldeki kuruluma girin. Eşleşmiş tarayıcı: {d.paired}</p>
        <div className="g7-button-row">
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.pair(), setCode)} data-kor-cmd="chromeExtension.pair">
            Eşleştirme kodu üret
          </button>
          {code ? (
            <>
              <code className="g7-mono" translate="no">{code.code}</code>
              <button type="button" className="btn small" aria-label="Kodu kopyala" title="Kodu kopyala" onClick={() => void navigator.clipboard.writeText(code.code)}>
                <Copy size={14} aria-hidden="true" />
              </button>
            </>
          ) : null}
          <span className="g9-grow" />
          <button type="button" className="btn small danger" onClick={() => run(api.chromeExtension.revoke(), () => setMessage('Tüm eşleşmeler kaldırıldı'))} data-kor-cmd="chromeExtension.revoke">
            Eşleşmeleri unut
          </button>
        </div>
      </section>

      <section className="g7-subsection">
        <h4 className="g7-h4">Tarayıcı araçlarının arka ucu</h4>
        <select className="g7-select" aria-label="Arka uç" value={d.backend} onChange={(e) => run(api.chromeExtension.setBackend(e.target.value))} data-kor-cmd="chromeExtension.setBackend">
          <option value="auto">Otomatik (eklenti bağlıysa tarayıcınız, değilse Kor'un yalıtık tarayıcısı)</option>
          <option value="chrome">Her zaman tarayıcınız (eklenti)</option>
          <option value="headless">Her zaman Kor'un yalıtık tarayıcısı</option>
        </select>
      </section>

      <section className="g7-subsection">
        <h4 className="g7-h4">Alan adları</h4>
        <p className="g9-small muted">Engellenen alan adlarında Kor hiçbir işlem yapmaz; izin listesi boş değilse yalnızca oradaki alan adlarında çalışır.</p>
        <div className="g7-inline">
          <input type="text" className="text-input" aria-label="Alan adı" placeholder="ornek.com" value={domain} onChange={(e) => setDomain(e.target.value)} />
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.allow(domain), () => setDomain(''))} data-kor-cmd="chromeExtension.allow">
            İzin ver
          </button>
          <button type="button" className="btn small" onClick={() => run(api.chromeExtension.deny(domain), () => setDomain(''))} data-kor-cmd="chromeExtension.deny">
            Engelle
          </button>
        </div>
        <DomainList title="İzinli" items={d.domains.allow} onRemove={(x) => run(api.chromeExtension.forget(x))} />
        <DomainList title="Engelli" items={d.domains.deny} onRemove={(x) => run(api.chromeExtension.forget(x))} />
      </section>

      <Profiles onError={setMessage} />
      <DocsCache onError={setMessage} />
      {message && <p className="g7-note" role="status">{message}</p>}
    </div>
  )
}

function DomainList({ title, items, onRemove }: { title: string; items: string[]; onRemove: (d: string) => void }) {
  if (items.length === 0) return null
  return (
    <div className="g9-small">
      {title}:
      {items.map((x) => (
        <span key={x} className="badge neutral" translate="no">
          {x}
          <button type="button" className="action-btn" aria-label={`${x} kaldır`} title={`${x} kaldır`} onClick={() => onRemove(x)} data-kor-cmd="chromeExtension.forget">
            <Trash2 size={12} aria-hidden="true" />
          </button>
        </span>
      ))}
    </div>
  )
}

const KEYS = ['full_name', 'given_name', 'family_name', 'email', 'phone', 'street', 'city', 'state', 'postal_code', 'country', 'company', 'job_title', 'website', 'github', 'linkedin', 'tax_id', 'tax_office']

function Profiles({ onError }: { onError: (m: string) => void }) {
  const list = useSWR('chrome:profiles', () => api.chromeExtension.profiles())
  const [name, setName] = useState('')
  const [rows, setRows] = useState<Array<[string, string]>>([])
  useEffect(() => {
    if (!name) return setRows([])
    api.chromeExtension.profileGet(name).then((p) => setRows(Object.entries(p)), (e: unknown) => onError(errorMessage(e)))
  }, [name])
  const save = () =>
    void api.chromeExtension
      .profileSet(name, Object.fromEntries(rows.filter(([k, v]) => k.trim() && v.trim())))
      .then(() => list.mutate(), (e: unknown) => onError(errorMessage(e)))
  const remove = () =>
    void api.chromeExtension.profileRemove(name).then(() => {
      setName('')
      void list.mutate()
    }, (e: unknown) => onError(errorMessage(e)))
  return (
    <section className="g7-subsection">
      <h4 className="g7-h4">Form doldurucu profilleri</h4>
      <p className="g9-small muted">Değerler işletim sisteminin anahtar zincirinde şifreli tutulur; hiçbir sağlayıcıya gönderilmez. Model yalnızca alan adlarını eşleştirir.</p>
      <div className="g7-inline">
        <select className="g7-select" aria-label="Profil" value={name} onChange={(e) => setName(e.target.value)} data-kor-cmd="chromeExtension.profiles">
          <option value="">— seçin —</option>
          {(list.data ?? []).map((p) => (
            <option key={p} value={p} translate="no">
              {p}
            </option>
          ))}
        </select>
        <input type="text" className="text-input" aria-label="Yeni profil adı" placeholder="yeni profil (ör. kisisel)" onKeyDown={(e) => e.key === 'Enter' && setName(e.currentTarget.value.trim())} />
      </div>
      {name ? (
        <div className="g7-stack" data-kor-cmd="chromeExtension.profileGet">
          {rows.map(([k, v], i) => (
            <div key={i} className="g7-inline">
              <input type="text" className="text-input" aria-label="Anahtar" list="kor-profile-keys" value={k} onChange={(e) => setRows(rows.map((r, j) => (j === i ? [e.target.value, r[1]] : r)))} />
              <input type="text" className="text-input" aria-label="Değer" value={v} onChange={(e) => setRows(rows.map((r, j) => (j === i ? [r[0], e.target.value] : r)))} />
            </div>
          ))}
          <datalist id="kor-profile-keys">
            {KEYS.map((k) => (
              <option key={k} value={k} />
            ))}
          </datalist>
          <div className="g7-button-row">
            <button type="button" className="btn small" onClick={() => setRows([...rows, ['', '']])}>
              <Plus size={14} aria-hidden="true" /> Alan ekle
            </button>
            <button type="button" className="btn small primary" onClick={save} data-kor-cmd="chromeExtension.profileSet">
              Kaydet
            </button>
            <button type="button" className="btn small danger" onClick={remove} data-kor-cmd="chromeExtension.profileRemove">
              Profili sil
            </button>
          </div>
        </div>
      ) : null}
    </section>
  )
}

function DocsCache({ onError }: { onError: (m: string) => void }) {
  const docs = useSWR('chrome:docs', () => api.chromeExtension.docsCache())
  return (
    <section className="g7-subsection">
      <h4 className="g7-h4">Okunan dokümantasyon sayfaları</h4>
      {(docs.data ?? []).length === 0 ? <p className="g9-small muted">Önbellek boş.</p> : null}
      <ul className="g9-small" data-kor-cmd="chromeExtension.docsCache">
        {(docs.data ?? []).map((x) => (
          <li key={x.url} translate="no">
            {x.title || x.url}
          </li>
        ))}
      </ul>
      <button
        type="button"
        className="btn small"
        onClick={() => void api.chromeExtension.docsCacheClear().then(() => docs.mutate(), (e: unknown) => onError(errorMessage(e)))}
        data-kor-cmd="chromeExtension.docsCacheClear"
      >
        Önbelleği boşalt
      </button>
    </section>
  )
}
