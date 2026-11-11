// Ayarlar > MCP sunucuları: dış araç sunucularını (ör. rea; Claude/Cursor ile aynı protokol) ekle, etkinleştir,
// bağlantıyı yenile ve sağlık durumunu gör. Sunucuların araçları tüm modellere normal araç gibi görünür (doğal dille
// kullanılır); varsayılan olarak onaya tabidir ve çıktıları güvenilmez veri kabul edilir.
import { Activity, Check, Code, KeyRound, Loader2, Plug, Plus, RefreshCw, Search, Sparkles, Store, Trash2 } from 'lucide-react'
import { useEffect } from 'react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { McpServerConfig } from '../../../core/mcp-config.ts'
import type { McpServerView } from '../../ipc-mcp.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { mcpServerSchema } from '../../../core/mcp-config.ts'
import { filterPopular, MCP_CATEGORIES, POPULAR_MCP_SERVERS, popularToConfig, type McpCategory, type PopularMcpServer } from './popular.ts'
import { PopularLogo } from './popular-logos.tsx'
import { GenerateForm, TestButton } from './GenerateForm.tsx'

/** MCP mağazası: arama + kategori + kartlar. Tıkla → (anahtar isteyen sunucuda kısa form) → kaydet + bağlan (spinner) → etkin. */
function McpStore({ views, onApply, onError }: { views: readonly McpServerView[]; onApply: (v: McpServerView[]) => void; onError: (m: string) => void }) {
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<McpCategory | 'all'>('all')
  const [busyId, setBusyId] = useState<string | null>(null)
  const [openForm, setOpenForm] = useState<string | null>(null)
  const [values, setValues] = useState<Record<string, string>>({})
  const installed = new Map(views.map((v) => [v.config.id, v] as const))
  const list = filterPopular(POPULAR_MCP_SERVERS, query, category)
  const sorted = [...list].sort((a, b) => Number(Boolean(b.featured)) - Number(Boolean(a.featured)))

  const install = (server: PopularMcpServer) => {
    if ((server.inputs?.length ?? 0) > 0 && openForm !== server.id) {
      setOpenForm(server.id)
      return
    }
    setBusyId(server.id)
    setOpenForm(null)
    void api.mcp
      .save(popularToConfig(server, values))
      .then((v) => {
        onApply(v)
        setValues({})
      })
      .catch((err: unknown) => onError(errorMessage(err)))
      .finally(() => setBusyId(null))
  }

  return (
    <section className="g7-subsection cdx-mcp-store">
      <div className="cdx-tags-head">
        <div>
          <h4 className="g7-h4"><Store size={15} aria-hidden="true" /> MCP mağazası</h4>
          <p className="g7-note">Tek tıkla ekle: sunucu kaydedilir, bağlantı hemen kurulur ve etkin gelir. Yerel sunucular ilk bağlantıda npx ile indirilir (Node gerekir).</p>
        </div>
        <div className="cdx-search cdx-mcp-search">
          <Search size={14} aria-hidden="true" />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Sunucu ara…" aria-label="MCP sunucusu ara" />
        </div>
      </div>
      <div className="cdx-mcp-cats" role="tablist" aria-label="Kategori">
        {MCP_CATEGORIES.map((c) => (
          <button key={c.id} type="button" role="tab" aria-selected={category === c.id} className={`cdx-mcp-cat${category === c.id ? ' on' : ''}`} onClick={() => setCategory(c.id)}>
            {c.label}
            <span className="cdx-mcp-cat-n">{c.id === 'all' ? POPULAR_MCP_SERVERS.length : POPULAR_MCP_SERVERS.filter((s) => s.category === c.id).length}</span>
          </button>
        ))}
      </div>
      {sorted.length === 0 && <p className="muted g9-small">Eşleşen sunucu yok.</p>}
      <div className="mcp-popular">
        {sorted.map((s) => {
          const view = installed.get(s.id)
          const busy = busyId === s.id
          const formOpen = openForm === s.id
          return (
            <div key={s.id} className={`mcp-popular-card${view ? ' installed' : ''}${busy ? ' busy' : ''}`}>
              <div className="mcp-popular-top">
                <PopularLogo logo={s.logo} />
                <span className="mcp-popular-body">
                  <span className="mcp-popular-name">
                    {s.name}
                    {s.featured && !view && <Sparkles size={12} className="cdx-mcp-star" aria-hidden="true" />}
                  </span>
                  <span className="mcp-popular-desc">{s.description}</span>
                  <span className="mcp-popular-tags">
                    <span className="badge neutral">{s.http ? 'HTTP' : 'yerel'}</span>
                    {s.inputs && s.inputs.length > 0 && <span className="badge neutral"><KeyRound size={10} aria-hidden="true" /> anahtar</span>}
                    {s.oauth && <span className="badge neutral">OAuth</span>}
                    {view && <StatusBadge view={view} />}
                  </span>
                </span>
              </div>
              {formOpen && (
                <form
                  className="cdx-mcp-inputs"
                  onSubmit={(e) => {
                    e.preventDefault()
                    install(s)
                  }}
                >
                  {s.inputs!.map((inp) => (
                    <label key={inp.key} className="cdx-mcp-input">
                      <span>{inp.label}</span>
                      <input className="text-input" type={inp.secret ? 'password' : 'text'} value={values[inp.key] ?? ''} onChange={(e) => setValues((v) => ({ ...v, [inp.key]: e.target.value }))} placeholder={inp.placeholder ?? inp.key} spellCheck={false} autoComplete="off" autoFocus={inp === s.inputs![0]} />
                    </label>
                  ))}
                  <div className="g7-button-row">
                    <button type="button" className="btn small" onClick={() => setOpenForm(null)}>Vazgeç</button>
                    <button type="submit" className="btn small primary"><Plus size={13} aria-hidden="true" /> Ekle ve etkinleştir</button>
                  </div>
                  <p className="g7-note">Değerler <code>~/.kor/mcp.json</code> dosyasına düz metin yazılır.</p>
                </form>
              )}
              <div className="mcp-popular-actions">
                {busy ? (
                  <span className="cdx-mcp-busy" role="status"><Loader2 size={14} className="cdx-spin" aria-hidden="true" /> Kuruluyor ve bağlanıyor…</span>
                ) : view ? (
                  <span className="cdx-mcp-done"><Check size={14} aria-hidden="true" /> Ekli{view.config.enabled ? ' · etkin' : ' · kapalı'}</span>
                ) : (
                  <button type="button" className="btn small primary" onClick={() => install(s)} disabled={busyId !== null}>
                    <Plus size={13} aria-hidden="true" /> {formOpen ? 'Anahtarı gir' : 'Ekle'}
                  </button>
                )}
              </div>
            </div>
          )
        })}
      </div>
    </section>
  )
}

const MCP_KEY = 'mcp:list'

function parseKv(text: string, sep: '=' | ':'): Record<string, string> {
  const out: Record<string, string> = {}
  for (const line of text.split('\n')) {
    const t = line.trim()
    if (!t) continue
    const i = t.indexOf(sep)
    if (i <= 0) throw new Error(`Satır "Ad${sep} değer" biçiminde olmalı: ${t}`)
    out[t.slice(0, i).trim()] = t.slice(i + 1).trim()
  }
  return out
}

function StatusBadge({ view }: { view: McpServerView }) {
  const { status } = view
  if (status.state === 'disabled') return <span className="badge neutral">Kapalı</span>
  if (status.state === 'connected') return <span className="badge g9-on">{status.toolCount} araç</span>
  return <span className="badge g9-health bad" title={status.error ?? ''}>{status.error ?? 'bağlantı hatası'}</span>
}

function ServerForm({ initial, onApply, onError }: { initial?: McpServerConfig; onApply: (v: McpServerView[]) => void; onError: (m: string) => void }) {
  const fid = useId()
  const editing = initial !== undefined
  const [transport, setTransport] = useState<'stdio' | 'http'>(initial?.transport ?? 'stdio')
  const [id, setId] = useState(initial?.id ?? '')
  const [label, setLabel] = useState(initial?.label ?? '')
  const [enabled, setEnabled] = useState(initial?.enabled ?? true)
  const [command, setCommand] = useState(initial?.transport === 'stdio' ? initial.command : '')
  const [args, setArgs] = useState(initial?.transport === 'stdio' ? initial.args.join(' ') : '')
  const [envText, setEnvText] = useState(initial?.transport === 'stdio' ? Object.entries(initial.env).map(([k, v]) => `${k}=${v}`).join('\n') : '')
  const [url, setUrl] = useState(initial?.transport === 'http' ? initial.url : '')
  const [headerText, setHeaderText] = useState(initial?.transport === 'http' ? Object.entries(initial.headers).map(([k, v]) => `${k}: ${v}`).join('\n') : '')

  const submit = (e: FormEvent) => {
    e.preventDefault()
    try {
      const common = { id: id.trim(), ...(label.trim() ? { label: label.trim() } : {}), enabled }
      const cfg: McpServerConfig =
        transport === 'stdio'
          ? { transport: 'stdio', ...common, command: command.trim(), args: args.trim() ? args.trim().split(/\s+/) : [], env: parseKv(envText, '=') }
          : { transport: 'http', ...common, url: url.trim(), headers: parseKv(headerText, ':') }
      void api.mcp
        .save(cfg)
        .then((v) => {
          onApply(v)
          if (!editing) {
            setId('')
            setLabel('')
            setCommand('')
            setArgs('')
            setEnvText('')
            setUrl('')
            setHeaderText('')
          }
        })
        .catch((err: unknown) => onError(errorMessage(err)))
    } catch (err) {
      onError(errorMessage(err))
    }
  }

  return (
    <form className="g9-form" onSubmit={submit}>
      <label htmlFor={`${fid}-t`}>Bağlantı türü</label>
      <select id={`${fid}-t`} className="g7-select" value={transport} onChange={(e) => setTransport(e.target.value as 'stdio' | 'http')} disabled={editing}>
        <option value="stdio">Yerel süreç (stdio)</option>
        <option value="http">Uzak (HTTP)</option>
      </select>

      <label htmlFor={`${fid}-id`}>Kimlik (araç adlarında kullanılır)</label>
      <input id={`${fid}-id`} className="text-input" value={id} onChange={(e) => setId(e.target.value)} placeholder="rea" maxLength={60} spellCheck={false} disabled={editing} />

      <label htmlFor={`${fid}-label`}>Etiket (isteğe bağlı)</label>
      <input id={`${fid}-label`} className="text-input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Tersine mühendislik" maxLength={80} />

      {transport === 'stdio' ? (
        <>
          <label htmlFor={`${fid}-cmd`}>Komut</label>
          <input id={`${fid}-cmd`} className="text-input" value={command} onChange={(e) => setCommand(e.target.value)} placeholder="rea-mcp veya npx" spellCheck={false} />
          <label htmlFor={`${fid}-args`}>Argümanlar (boşlukla ayrılır)</label>
          <input id={`${fid}-args`} className="text-input" value={args} onChange={(e) => setArgs(e.target.value)} placeholder="-y @kapsam/sunucu" spellCheck={false} />
          <label htmlFor={`${fid}-env`}>Ortam değişkenleri (satır başına "AD=değer")</label>
          <textarea id={`${fid}-env`} className="text-input g9-textarea" rows={2} value={envText} onChange={(e) => setEnvText(e.target.value)} spellCheck={false} />
        </>
      ) : (
        <>
          <label htmlFor={`${fid}-url`}>Sunucu adresi</label>
          <input id={`${fid}-url`} className="text-input" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://sunucu.example.com/mcp" spellCheck={false} />
          <label htmlFor={`${fid}-hdr`}>Başlıklar (satır başına "Ad: değer")</label>
          <textarea id={`${fid}-hdr`} className="text-input g9-textarea" rows={2} value={headerText} onChange={(e) => setHeaderText(e.target.value)} spellCheck={false} />
        </>
      )}

      {!editing && (
        <label className="g9-check">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Ekledikten sonra etkinleştir
        </label>
      )}
      <div className="g7-button-row">
        <button type="submit" className="btn small" disabled={!id.trim() || (transport === 'stdio' ? !command.trim() : !url.trim())}>
          {editing ? 'Kaydet' : (<><Plus size={14} aria-hidden="true" /> Sunucu ekle</>)}
        </button>
      </div>
    </form>
  )
}

function ServerCard({ view, onApply, onError }: { view: McpServerView; onApply: (v: McpServerView[]) => void; onError: (m: string) => void }) {
  const titleId = useId()
  const { config, status } = view
  const toggle = (enabled: boolean) => void api.mcp.setEnabled(config.id, enabled).then(onApply).catch((err: unknown) => onError(errorMessage(err)))
  const remove = () =>
    void confirmAction({ title: 'Sunucuyu sil', message: `"${config.label ?? config.id}" MCP sunucusu kaldırılsın mı? Araçları sohbetlerden çıkar.`, confirmLabel: 'Sil', danger: true }).then((ok) => {
      if (ok) void api.mcp.remove(config.id).then(onApply).catch((err: unknown) => onError(errorMessage(err)))
    })

  return (
    <section className="g7-subsection g9-provider" aria-labelledby={titleId}>
      <h4 id={titleId} className="g7-h4">
        {config.label ?? config.id}
        <span className="badge neutral">{config.transport === 'stdio' ? 'yerel' : 'HTTP'}</span>
        <StatusBadge view={view} />
        <span className="g9-grow" />
        <label className="g9-check" title="Etkinleştir / kapat">
          <input type="checkbox" checked={config.enabled} onChange={(e) => toggle(e.target.checked)} /> Etkin
        </label>
        {config.transport === 'stdio' && config.enabled && <TestButton id={config.id} label={config.label ?? config.id} />}
        <button type="button" className="action-btn" onClick={remove} aria-label={`${config.label ?? config.id} sunucusunu sil`} title="Sil">
          <Trash2 size={14} aria-hidden="true" />
        </button>
      </h4>
      <p className="g7-mono g9-url">{status.target}</p>
      {status.state === 'error' && status.error && <p className="error-text g9-small">{status.error}</p>}
      <details className="g9-details">
        <summary>Düzenle</summary>
        <ServerForm initial={config} onApply={onApply} onError={onError} />
      </details>
    </section>
  )
}

/** cdesktop MCP JSON düzenleyici: mcp.json dizisi metin olarak; değişiklik varsa altta "Vazgeç | Kaydet" şeridi. */
function JsonEditor({ servers, onApply, onError }: { servers: readonly McpServerConfig[]; onApply: (v: McpServerView[]) => void; onError: (m: string) => void }) {
  const original = JSON.stringify(servers, null, 2)
  const [text, setText] = useState(original)
  const [busy, setBusy] = useState(false)
  useEffect(() => setText(original), [original])
  const dirty = text !== original
  const save = async () => {
    setBusy(true)
    try {
      const parsed: unknown = JSON.parse(text)
      if (!Array.isArray(parsed)) throw new Error('Kök bir dizi olmalı: [ { "transport": "stdio", "id": "...", ... } ]')
      const next = parsed.map((raw, i) => {
        const r = mcpServerSchema.safeParse(raw)
        if (!r.success) throw new Error(`${i + 1}. sunucu geçersiz: ${r.error.issues.map((x) => x.message).join('; ')}`)
        return r.data
      })
      if (new Set(next.map((s) => s.id)).size !== next.length) throw new Error('Sunucu kimlikleri benzersiz olmalı')
      let view: McpServerView[] = []
      for (const old of servers) if (!next.some((n) => n.id === old.id)) view = await api.mcp.remove(old.id)
      for (const cfg of next) view = await api.mcp.save(cfg)
      onApply(view)
    } catch (err) {
      onError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="cdx-mcp-json">
      <textarea className="text-input g9-textarea cdx-mcp-json-input" value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} aria-label="mcp.json" rows={14} />
      <p className="g7-note">Değişiklikler şuraya kaydedilir: <code>~/.kor/mcp.json</code></p>
      {dirty && (
        <div className="cdx-unsaved">
          <span>Kaydedilmemiş değişiklikler</span>
          <button type="button" className="btn small" onClick={() => setText(original)} disabled={busy}>Vazgeç</button>
          <button type="button" className="btn small primary" onClick={() => void save()} disabled={busy}>{busy ? 'Kaydediliyor…' : 'Kaydet'}</button>
        </div>
      )}
    </div>
  )
}

export function McpSection() {
  const [editMode, setEditMode] = useState<'form' | 'json'>('form')
  const { data, error, mutate } = useSWR(MCP_KEY, () => api.mcp.list(), { revalidateOnFocus: false })
  const [message, setMessage] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const apply = (next: McpServerView[]) => {
    setMessage(null)
    void mutate(next, { revalidate: false })
  }
  const reconnect = () => {
    setBusy(true)
    setMessage(null)
    void api.mcp
      .reconnect()
      .then((v) => void mutate(v, { revalidate: false }))
      .catch((err: unknown) => setMessage(errorMessage(err)))
      .finally(() => setBusy(false))
  }

  if (error) return <p className="error-text">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>

  return (
    <div className="g7-stack">
      <p className="g7-note text-pretty">
        <Plug size={12} aria-hidden="true" /> MCP sunucularının araçları tüm modellere normal araç gibi görünür ve doğal dille kullanılır. Yalnızca etkin sunuculara
        bağlanılır. Yerel sunucular uygulamanın API anahtarları olmadan başlatılır; araç çıktıları güvenilmez veri kabul edilir ve araçlar varsayılan olarak onaya tabidir.
        Yapılandırma ~/.kor/mcp.json dosyasında (ortam değişkenleri ve başlıklar düz metin durur; sır için dikkatli ol).
      </p>
      <div className="g7-button-row">
        <button type="button" className="btn small" onClick={reconnect} disabled={busy}>
          <RefreshCw size={14} aria-hidden="true" /> {busy ? 'Bağlanıyor...' : 'Yeniden bağlan'}
        </button>
        <span className="g9-grow" />
        <label className="g9-check" title="mcp.json'u doğrudan düzenle (cdesktop Edit JSON)">
          <input type="checkbox" checked={editMode === 'json'} onChange={(e) => setEditMode(e.target.checked ? 'json' : 'form')} /> <Code size={13} aria-hidden="true" /> JSON düzenle
        </label>
      </div>
      {editMode === 'json' && <JsonEditor servers={data.map((v) => v.config)} onApply={apply} onError={setMessage} />}
      {message && <p className="error-text" role="status">{message}</p>}
      {data.length === 0 && (
        <p className="muted">
          <Activity size={14} aria-hidden="true" /> Henüz MCP sunucusu yok. Mağazadan tek tıkla ekle ya da aşağıdan elle tanımla.
        </p>
      )}
      {data.map((v) => (
        <ServerCard key={v.config.id} view={v} onApply={apply} onError={setMessage} />
      ))}
      <McpStore views={data} onApply={apply} onError={setMessage} />
      <section className="g7-subsection">
        <h4 className="g7-h4">Sunucu ekle</h4>
        <ServerForm onApply={apply} onError={setMessage} />
      </section>
      <section className="g7-subsection">
        <h4 className="g7-h4">Kaynaktan üret: veritabanı, OpenAPI, GraphQL, klasör</h4>
        <GenerateForm onDone={() => void mutate()} onError={setMessage} />
      </section>
    </div>
  )
}
