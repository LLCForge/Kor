// Ayarlar > MCP: veritabanından / OpenAPI'den / GraphQL'den / klasörden sunucu üret (`kor mcp add <kaynak>`)
// ve bir sunucuyu sına (`kor mcp test`). Üretim çekirdekte (kor_agent::mcp_gen); burada yalnızca form.
import { FlaskConical, Loader2, Wand2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import type { McpGenApi, McpGenKind, McpGenRequest, McpTestReport } from '../../ipc-mcpgen.ts'
import { api, errorMessage } from '../api.ts'

export const mcpGenApi = (api as unknown as { mcpgen: McpGenApi }).mcpgen

/** Test raporunun tek satırlık özeti. */
export function testSummary(r: McpTestReport): string {
  if (r.error) return r.error
  const head = `${r.connectMs} ms'de başladı, ${r.tools.length} araç (${r.listMs} ms)`
  if (!r.probe) return `${head}; argümansız salt okunur araç yok, hiçbir şey çağrılmadı`
  return `${head}; ${r.probe}: ${r.probeOk ? 'tamam' : 'hata'} (${r.probeMs} ms)`
}

const SOURCES: { kind: McpGenKind; label: string; placeholder: string }[] = [
  { kind: 'database', label: 'Veritabanı', placeholder: 'postgres://kullanici:parola@sunucu/vt · mysql://… · sqlite:veri.db' },
  { kind: 'openapi', label: 'OpenAPI', placeholder: 'https://api.example.com/openapi.json ya da ./openapi.yaml' },
  { kind: 'graphql', label: 'GraphQL', placeholder: 'https://api.example.com/graphql ya da ./schema.graphql' },
  { kind: 'local', label: 'Klasör', placeholder: './benim-servisim' },
]

/** Kaynaktan sunucu üretir; bitince `onDone` listeyi yeniler. */
export function GenerateForm({ onDone, onError }: { onDone: () => void; onError: (m: string) => void }) {
  const fid = useId()
  const [kind, setKind] = useState<McpGenKind>('database')
  const [source, setSource] = useState('')
  const [id, setId] = useState('')
  const [baseUrl, setBaseUrl] = useState('')
  const [auth, setAuth] = useState('')
  const [allowWrites, setAllowWrites] = useState(false)
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState<string | null>(null)
  const current = SOURCES.find((s) => s.kind === kind) ?? SOURCES[0]

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const req: McpGenRequest = { source: source.trim(), kind, allowWrites }
    if (id.trim()) req.id = id.trim()
    if (baseUrl.trim()) req.baseUrl = baseUrl.trim()
    if (auth.trim()) req.auth = auth.trim()
    setBusy(true)
    setDone(null)
    void mcpGenApi
      .add(req)
      .then((r) => {
        setSource('')
        setId('')
        setDone(`"${r.id}" eklendi: ${r.tools} araç${r.note ? ` (${r.note})` : ''}`)
        onDone()
      })
      .catch((err: unknown) => onError(errorMessage(err)))
      .finally(() => setBusy(false))
  }

  return (
    <form className="g9-form" onSubmit={submit}>
      <label htmlFor={`${fid}-k`}>Kaynak türü</label>
      <select id={`${fid}-k`} className="g7-select" value={kind} onChange={(e) => setKind(e.target.value as McpGenKind)}>
        {SOURCES.map((s) => (
          <option key={s.kind} value={s.kind}>{s.label}</option>
        ))}
      </select>
      <label htmlFor={`${fid}-s`}>Kaynak</label>
      <input id={`${fid}-s`} className="text-input" value={source} onChange={(e) => setSource(e.target.value)} placeholder={current.placeholder} maxLength={4096} spellCheck={false} translate="no" />
      {kind === 'database' && <p className="g7-note">Adresteki parola dosyaya yazılmaz, anahtar zincirine kaydedilir.</p>}
      <label htmlFor={`${fid}-id`}>Kimlik (isteğe bağlı; küçük harf, rakam, -)</label>
      <input id={`${fid}-id`} className="text-input" value={id} onChange={(e) => setId(e.target.value)} maxLength={40} spellCheck={false} />
      {(kind === 'openapi' || kind === 'graphql') && (
        <>
          <label htmlFor={`${fid}-b`}>Taban adres / uç nokta (şema dosyası için)</label>
          <input id={`${fid}-b`} className="text-input" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://api.example.com" maxLength={2048} spellCheck={false} />
          <label htmlFor={`${fid}-a`}>Kimlik doğrulama (kasadan: bearer:SIR, header:Ad:SIR, query:ad:SIR)</label>
          <input id={`${fid}-a`} className="text-input" value={auth} onChange={(e) => setAuth(e.target.value)} placeholder="bearer:API_TOKEN" maxLength={300} spellCheck={false} />
        </>
      )}
      {kind !== 'local' && (
        <label className="g9-check">
          <input type="checkbox" checked={allowWrites} onChange={(e) => setAllowWrites(e.target.checked)} /> Veri değiştiren araçları da üret (her çağrı onay ister)
        </label>
      )}
      <div className="g7-button-row">
        <button type="submit" className="btn small" disabled={busy || !source.trim()} data-kor-cmd="mcpgen.add">
          {busy ? <Loader2 size={14} className="spin" aria-hidden="true" /> : <Wand2 size={14} aria-hidden="true" />} Sunucu üret
        </button>
      </div>
      {done && <p className="g7-note" role="status" translate="no">{done}</p>}
    </form>
  )
}

/** Sunucu kartındaki "Dene" düğmesi ve sonucu. */
export function TestButton({ id, label }: { id: string; label: string }) {
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null)
  const run = () => {
    setBusy(true)
    setResult(null)
    void mcpGenApi
      .test(id)
      .then((r) => setResult({ ok: !r.error && (r.probe === null || r.probeOk), text: testSummary(r) }))
      .catch((err: unknown) => setResult({ ok: false, text: errorMessage(err) }))
      .finally(() => setBusy(false))
  }
  return (
    <>
      <button type="button" className="action-btn" onClick={run} disabled={busy} aria-label={`${label} sunucusunu dene`} title="Dene: bağlan, araçları listele, salt okunur bir aracı çağır" data-kor-cmd="mcpgen.test">
        {busy ? <Loader2 size={14} className="spin" aria-hidden="true" /> : <FlaskConical size={14} aria-hidden="true" />}
      </button>
      {result && (
        <span className={result.ok ? 'badge g9-on' : 'badge g9-health bad'} role="status" translate="no">
          {result.text}
        </span>
      )}
    </>
  )
}
