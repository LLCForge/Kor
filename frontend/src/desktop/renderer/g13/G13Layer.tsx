// [g13] Geliştirici araçları: katmanlı sistem istemi, .env yöneticisi, doküman kaynakları, bağlam köprüsü
// ve sohbet genelinde hata açıklayıcı / yığın izi gezgini. App.tsx'e tek bileşen olarak bağlanır.
import { useEffect, useId, useState, type Dispatch, type FormEvent, type SetStateAction } from 'react'
import useSWR from 'swr'
import type { ExportFormat } from '../../ipc-g13.ts'
import { api, errorMessage } from '../api.ts'
import { CopyButton } from '../CopyButton.tsx'
import { G7Modal } from '../G7Modal.tsx'
import { G13_OPEN_EVENT, type G13Tab } from './bus.ts'
import { EnvPanel } from './EnvManager.tsx'
import { ErrorAssist } from './ErrorAssist.tsx'
import { uiLocale } from '../ui-locale.ts'

const TABS: ReadonlyArray<{ id: G13Tab; label: string }> = [
  { id: 'layers', label: 'Sistem istemi katmanları' },
  { id: 'env', label: 'Ortam değişkenleri' },
  { id: 'docs', label: 'Doküman kaynakları' },
  { id: 'export', label: 'Bağlamı dışa aktar' },
]
const MAX_LAYER_CHARS = 16_000

const isTab = (v: unknown): v is G13Tab => TABS.some((t) => t.id === v)

function LayerEditor({ label, initial, placeholder, onSave }: { label: string; initial: string; placeholder: string; onSave: (text: string) => Promise<void> }) {
  const [text, setText] = useState(initial)
  const [state, setState] = useState<{ saving: boolean; error: string | null }>({ saving: false, error: null })
  const id = useId()
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setState({ saving: true, error: null })
    try {
      await onSave(text)
      setState({ saving: false, error: null })
    } catch (err) {
      setState({ saving: false, error: errorMessage(err) })
    }
  }
  return (
    <form className="g13-stack" onSubmit={(e) => void submit(e)}>
      <label className="sr-only" htmlFor={id}>{label}</label>
      <textarea id={id} className="dialog-textarea g13-layer-text" value={text} placeholder={placeholder} onChange={(e) => setText(e.target.value)} spellCheck={false} />
      <div className="g13-row g13-row-end">
        <span className="dialog-meta">{text.length.toLocaleString(uiLocale())} / {MAX_LAYER_CHARS.toLocaleString(uiLocale())}</span>
        <button type="submit" className="btn small primary" disabled={state.saving || text === initial || text.length > MAX_LAYER_CHARS}>
          {state.saving ? 'Kaydediliyor...' : 'Kaydet'}
        </button>
      </div>
      {state.error && <p className="error-text" role="alert">{state.error}</p>}
    </form>
  )
}

function LayersPanel({ sessionId, onOpenGlobalPrompt }: { sessionId: string; onOpenGlobalPrompt: () => void }) {
  const { data, error, mutate } = useSWR(['g13:layers', sessionId], () => api.g13.getLayers(sessionId))
  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>
  const approxTokens = Math.round(data.preview.length / 4)

  return (
    <div className="g13-stack">
      <p className="dialog-desc text-pretty">
        Katmanlar sırayla birleşir; sonraki katman öncekini daraltır: global, proje, oturum. Ortam bilgisi, AGENTS.md ve hafıza global katmana eklenir.
      </p>

      <section className="g13-section" aria-labelledby="g13-layer-global">
        <div className="g13-row">
          <h3 id="g13-layer-global" className="g13-h">1. Global</h3>
          <span className="badge neutral">{data.global.custom ? 'Özel istem' : 'Varsayılan'}</span>
          <span className="dialog-meta">{data.global.chars.toLocaleString(uiLocale())} karakter</span>
          <button type="button" className="btn small g13-push" onClick={onOpenGlobalPrompt}>Düzenle</button>
        </div>
      </section>

      <section className="g13-section" aria-labelledby="g13-layer-project">
        <div className="g13-row">
          <h3 id="g13-layer-project" className="g13-h">2. Proje</h3>
          <code>{data.project.file}</code>
        </div>
        {!data.project.trusted && (
          <p className="g13-warn" role="status">
            Bu klasör güvenilir olarak işaretlenmedi: proje katmanı ve sürüm bilgisi isteme eklenmiyor. Kararını Ayarlar &gt; İzinler bölümünden değiştirebilirsin.
          </p>
        )}
        <LayerEditor
          key={`project:${data.root}:${data.project.text}`}
          label="Proje sistem istemi"
          initial={data.project.text}
          placeholder="Depoyla paylaşılan proje talimatları (ör. kod stili, mimari kurallar)"
          onSave={async (text) => {
            await api.g13.setProjectPrompt(sessionId, text)
            await mutate()
          }}
        />
      </section>

      <section className="g13-section" aria-labelledby="g13-layer-session">
        <h3 id="g13-layer-session" className="g13-h">3. Oturum</h3>
        <LayerEditor
          key={`session:${sessionId}:${data.session.text}`}
          label="Oturum sistem istemi"
          initial={data.session.text}
          placeholder="Yalnızca bu sohbet için talimat (ör. bu sohbette yalnızca testlere dokun)"
          onSave={async (text) => {
            await api.g13.setSessionPrompt(sessionId, text)
            await mutate()
          }}
        />
      </section>

      <section className="g13-section" aria-labelledby="g13-layer-preview">
        <div className="g13-row">
          <h3 id="g13-layer-preview" className="g13-h">Birleşik önizleme</h3>
          <span className="dialog-meta">
            {data.preview.length.toLocaleString(uiLocale())} karakter, yaklaşık {approxTokens.toLocaleString(uiLocale())} token
          </span>
          <CopyButton getText={() => data.preview} label="Önizlemeyi kopyala" className="action-btn g13-push" />
        </div>
        <pre className="g13-preview" tabIndex={0} aria-label="Birleşik sistem istemi">{data.preview}</pre>
      </section>
    </div>
  )
}

function DocsPanel() {
  const { data, error, mutate } = useSWR('g13:docs', () => api.g13.getDocSources())
  const [input, setInput] = useState('')
  const [status, setStatus] = useState<string | null>(null)
  const id = useId()

  const save = async (next: string[]) => {
    setStatus(null)
    try {
      await api.g13.setDocSources(next)
      await mutate()
      return true
    } catch (err) {
      setStatus(errorMessage(err))
      return false
    }
  }
  const add = async (e: FormEvent) => {
    e.preventDefault()
    const url = input.trim()
    if (!/^https?:\/\/[^\s/?#]+[^\s?#]*$/.test(url)) return setStatus('http(s) ile başlayan, sorgu (?) ve # içermeyen bir adres öneki gir.')
    if (await save([...(data ?? []), url])) setInput('')
  }

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>
  return (
    <div className="g13-stack">
      <p className="dialog-desc text-pretty">
        docs_fetch aracı yalnızca bu adres öneklerinden (ör. https://react.dev/reference/) güncel doküman okur ve sonucu 12.000 karakterle sınırlar.
        Her istek yine izin moduna göre onaya tabidir. Liste yalnızca kullanıcı ayar dosyasında tutulur; proje dosyaları kaynak ekleyemez.
      </p>
      {data.length === 0 ? (
        <p className="muted">Onaylı kaynak yok.</p>
      ) : (
        <ul className="g13-list">
          {data.map((src) => (
            <li key={src} className="g13-row">
              <code className="g13-grow">{src}</code>
              <button type="button" className="btn small ghost" onClick={() => void save(data.filter((s) => s !== src))} aria-label={`${src} kaynağını kaldır`}>
                Kaldır
              </button>
            </li>
          ))}
        </ul>
      )}
      <form className="g13-row" onSubmit={(e) => void add(e)}>
        <label className="sr-only" htmlFor={id}>Doküman kaynağı</label>
        <input id={id} className="text-input g13-grow" placeholder="https://docs.python.org/3/" value={input} onChange={(e) => setInput(e.target.value)} spellCheck={false} />
        <button type="submit" className="btn small" disabled={!input.trim()}>Ekle</button>
      </form>
      {status && <p className="error-text" role="alert">{status}</p>}
    </div>
  )
}

function ExportPanel({ sessionId }: { sessionId: string }) {
  const [format, setFormat] = useState<ExportFormat>('markdown')
  const [text, setText] = useState<string | null>(null)
  const [status, setStatus] = useState<{ error: boolean; message: string } | null>(null)
  const [busy, setBusy] = useState(false)

  const run = async (fn: () => Promise<void>) => {
    setBusy(true)
    setStatus(null)
    try {
      await fn()
    } catch (err) {
      setStatus({ error: true, message: errorMessage(err) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="g13-stack">
      <p className="dialog-desc text-pretty">
        Görevi başka bir araçta sürdürmek için ilk istem, son istek, özet ve dosya listesinden taşınabilir bir paket. Sırlar maskelenir;
        sistem istemi ve araç çıktıları pakete girmez.
      </p>
      <fieldset className="g13-row g13-fieldset">
        <legend className="sr-only">Biçim</legend>
        {(['markdown', 'json'] as const).map((f) => (
          <label key={f} className="g13-radio">
            <input type="radio" name="g13-export-format" value={f} checked={format === f} onChange={() => setFormat(f)} />
            {f === 'markdown' ? 'Markdown' : 'JSON'}
          </label>
        ))}
        <button type="button" className="btn small primary g13-push" disabled={busy} onClick={() => void run(async () => setText(await api.g13.buildExport(sessionId, format)))}>
          Oluştur
        </button>
        <button
          type="button"
          className="btn small"
          disabled={busy}
          onClick={() =>
            void run(async () => {
              const saved = await api.g13.saveExport(sessionId, format)
              if (saved) setStatus({ error: false, message: `Kaydedildi: ${saved}` })
            })
          }
        >
          Dosyaya kaydet
        </button>
      </fieldset>
      {text !== null && (
        <>
          <div className="g13-row g13-row-end">
            <CopyButton getText={() => text} label="Paketi kopyala" />
          </div>
          <textarea className="dialog-textarea g13-export-text" readOnly value={text} aria-label="Bağlam paketi" spellCheck={false} />
        </>
      )}
      {status && (
        <p className={status.error ? 'error-text' : 'muted'} role={status.error ? 'alert' : 'status'}>
          {status.message}
        </p>
      )}
    </div>
  )
}

interface G13LayerProps {
  sessionId: string
  root: string | null
  setDraft: Dispatch<SetStateAction<string>>
  openFile: (path: string, line?: number) => void
  openGlobalPrompt: () => void
}

export function G13Layer({ sessionId, root, setDraft, openFile, openGlobalPrompt }: G13LayerProps) {
  const [tab, setTab] = useState<G13Tab | null>(null)
  const titleId = useId()

  useEffect(() => {
    const onOpen = (e: Event) => setTab(e instanceof CustomEvent && isTab(e.detail) ? e.detail : 'layers')
    window.addEventListener(G13_OPEN_EVENT, onOpen)
    return () => window.removeEventListener(G13_OPEN_EVENT, onOpen)
  }, [])

  const explain = (prompt: string) => {
    setDraft((d) => (d.trim() ? `${d}\n\n${prompt}` : prompt))
    requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>('textarea.composer-input')?.focus())
  }

  return (
    <>
      <ErrorAssist root={root} onExplain={explain} onOpenFile={openFile} />
      <G7Modal open={tab !== null} onClose={() => setTab(null)} titleId={titleId} title="Geliştirici araçları" className="g13-modal">
        <div className="g13-body">
          <div className="g7-tabs" role="tablist" aria-label="Geliştirici araçları bölümleri">
            {TABS.map((t) => (
              <button key={t.id} type="button" role="tab" className="g7-tab" aria-selected={tab === t.id} onClick={() => setTab(t.id)}>
                {t.label}
              </button>
            ))}
          </div>
          <div role="tabpanel" aria-label={TABS.find((t) => t.id === tab)?.label} className="g13-panel">
            {!root ? (
              <p className="muted">Önce bir klasör aç.</p>
            ) : tab === 'layers' ? (
              <LayersPanel
                sessionId={sessionId}
                onOpenGlobalPrompt={() => {
                  setTab(null)
                  openGlobalPrompt()
                }}
              />
            ) : tab === 'env' ? (
              <EnvPanel root={root} />
            ) : tab === 'docs' ? (
              <DocsPanel />
            ) : tab === 'export' ? (
              <ExportPanel key={sessionId} sessionId={sessionId} />
            ) : null}
          </div>
        </div>
      </G7Modal>
    </>
  )
}
