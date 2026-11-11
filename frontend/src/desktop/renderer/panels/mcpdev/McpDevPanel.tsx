// §28.15 MCP denetleyicisi (ADR 0063): geliştirilen bir MCP sunucusunu çalıştırır, dosyaları değişince
// yeniden yükler ve araç şemasındaki farkı gösterir, araç/kaynak/istemleri listeler, araçları elle
// çağırır, JSON-RPC trafiğini canlı ve hataları açıklamalı gösterir. CLI: kor mcp dev ./sunucu.
// Ajan da bağlı bir sunucuyu onayla yeniden başlatabilir (`mcp_restart`).
import { Play, RotateCw, Send, Square } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import type { McpInventory, McpSchemaDiff, McpTraffic } from '../../../ipc-ek14.ts'
import { errorMessage } from '../../api.ts'
import { mcpDevApi } from '../../ek14-api.ts'
import type { KorPanelProps } from '../registry.ts'
import { append, summarize } from './traffic.ts'

function DiffView({ d }: { d: McpSchemaDiff }) {
  if (d.added.length + d.removed.length + d.changed.length === 0) return <p className="ks-ok">Yeniden yüklendi; araç şemaları değişmedi.</p>
  return (
    <ul className="ks-list" aria-label="Şema farkı">
      {d.added.map((n) => <li key={`a${n}`} className="stat-add" translate="no">+ {n}</li>)}
      {d.removed.map((n) => <li key={`r${n}`} className="stat-del" translate="no">− {n}</li>)}
      {d.changed.map(([n, w]) => <li key={`c${n}`} className="ks-warn" translate="no">~ {n}: {w}</li>)}
    </ul>
  )
}

export function McpDevPanel(_: KorPanelProps) {
  const uid = useId()
  const [path, setPath] = useState('')
  const [command, setCommand] = useState('')
  const [id, setId] = useState<number | null>(null)
  const [inv, setInv] = useState<McpInventory | null>(null)
  const [diff, setDiff] = useState<McpSchemaDiff | null>(null)
  const [log, setLog] = useState<McpTraffic[]>([])
  const [tool, setTool] = useState('')
  const [args, setArgs] = useState('{}')
  const [result, setResult] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (id === null) return
    const off1 = mcpDevApi.onTraffic((e) => {
      if (e.id === id) setLog((l) => append(l, e.traffic))
    })
    const off2 = mcpDevApi.onReloaded((e) => {
      if (e.id !== id) return
      if (e.error) setErr(e.error)
      if (e.inventory) setInv(e.inventory)
      if (e.diff) setDiff(e.diff)
    })
    return () => {
      off1()
      off2()
    }
  }, [id])

  const start = (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setErr(null)
    setLog([])
    mcpDevApi
      .start(path.trim(), command.trim() || null)
      .then((r) => {
        setId(r.id)
        setInv(r.inventory)
        setTool(r.inventory.tools[0]?.name ?? '')
      })
      .catch((x: unknown) => setErr(errorMessage(x)))
      .finally(() => setBusy(false))
  }
  const call = (e: FormEvent) => {
    e.preventDefault()
    if (id === null || !tool) return
    setResult(null)
    mcpDevApi.call(id, tool, args).then((r) => setResult(JSON.stringify(r, null, 2))).catch((x: unknown) => setResult(errorMessage(x)))
  }
  const stop = () => {
    if (id !== null) void mcpDevApi.stop(id)
    setId(null)
    setInv(null)
    setDiff(null)
  }
  const name = (inv?.server as { serverInfo?: { name?: string; version?: string } } | undefined)?.serverInfo

  return (
    <section className="ks" aria-label="MCP geliştirici modu">
      <h2 className="ks-title">MCP geliştirici modu</h2>
      {id === null ? (
        <form className="ks-form" onSubmit={start}>
          <label className="ks-field" htmlFor={`${uid}-path`}>Sunucu dosyası ya da klasörü</label>
          <input id={`${uid}-path`} className="text-input" value={path} onChange={(e) => setPath(e.target.value)} placeholder="C:\\projeler\\my-mcp" translate="no" />
          <label className="ks-field" htmlFor={`${uid}-cmd`}>Komut (algılanamazsa, ör. node build/index.js)</label>
          <input id={`${uid}-cmd`} className="text-input" value={command} onChange={(e) => setCommand(e.target.value)} translate="no" />
          <button type="submit" className="btn primary" disabled={busy || !path.trim()} data-kor-cmd="mcpdev.start"><Play size={14} aria-hidden="true" /> {busy ? 'Başlıyor…' : 'Başlat'}</button>
        </form>
      ) : (
        <div className="kg-card-head" data-kor-event="kor:mcpdev/traffic">
          <strong translate="no">{name?.name ?? 'sunucu'} {name?.version ?? ''}</strong>
          <button type="button" className="action-btn" onClick={() => mcpDevApi.reload(id).then((r) => { setInv(r.inventory); setDiff(r.diff) }).catch((x: unknown) => setErr(errorMessage(x)))} aria-label="Sunucuyu yeniden yükle" title="Yeniden yükle" data-kor-cmd="mcpdev.reload">
            <RotateCw size={16} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn reject" onClick={stop} aria-label="Sunucuyu durdur" title="Durdur" data-kor-cmd="mcpdev.stop"><Square size={16} aria-hidden="true" /></button>
        </div>
      )}
      {err && <p className="error-text" role="alert">{err}</p>}
      {diff && <div data-kor-event="kor:mcpdev/reloaded"><DiffView d={diff} /></div>}
      {inv && (
        <>
          <p className="ks-label">{inv.tools.length} araç · {inv.resources.length} kaynak · {inv.prompts.length} istem</p>
          <ul className="ks-list" data-kor-cmd="mcpdev.inspect">
            {inv.tools.map((t) => (
              <li key={t.name} className="ks-row">
                <code translate="no">{t.name}({Object.keys(t.inputSchema?.properties ?? {}).join(', ')})</code>
                <span className="kg-link-meta" translate="no">{t.description?.split('\n')[0] ?? ''}</span>
              </li>
            ))}
            {inv.resources.map((r) => <li key={r.uri} className="ks-row"><span className="ks-chip">kaynak</span><code translate="no">{r.uri}</code></li>)}
            {inv.prompts.map((p) => <li key={p.name} className="ks-row"><span className="ks-chip">istem</span><code translate="no">{p.name}</code></li>)}
          </ul>
          <form className="ks-form" onSubmit={call}>
            <label className="ks-field" htmlFor={`${uid}-tool`}>Aracı çağır</label>
            <select id={`${uid}-tool`} className="g7-select" value={tool} onChange={(e) => setTool(e.target.value)}>
              {inv.tools.map((t) => <option key={t.name} value={t.name}>{t.name}</option>)}
            </select>
            <textarea className="text-input ks-textarea" aria-label="Argümanlar (JSON)" value={args} onChange={(e) => setArgs(e.target.value)} translate="no" />
            <button type="submit" className="btn" disabled={!tool} data-kor-cmd="mcpdev.call"><Send size={14} aria-hidden="true" /> Çağır</button>
          </form>
          {result && <pre className="ks-code" translate="no">{result}</pre>}
        </>
      )}
      {log.length > 0 && (
        <details className="ks-more" open>
          <summary>JSON-RPC trafiği ({log.length})</summary>
          <ol className="ks-list" aria-live="polite">
            {log.slice(-200).map((t, i) => (
              <li key={`${t.ms}-${i}`} className={`ks-entry-text${t.dir === 'note' ? ' ks-warn' : ''}`} title={t.text} translate="no">
                {t.ms} ms · {summarize(t)}
              </li>
            ))}
          </ol>
        </details>
      )}
    </section>
  )
}
