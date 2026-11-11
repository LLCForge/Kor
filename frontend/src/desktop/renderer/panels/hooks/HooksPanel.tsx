// §28.12 görsel hook editörü (ADR 0063): Kor hooks.toml (kullanıcı/proje) ve Claude Code settings.json
// (kullanıcı/proje/yerel) hook'larını listeler, ekler, düzenler, siler; olay seçici, eylem (komut / MCP
// çağrısı / bildirim), koşullar (dosya deseni, dal, model), örnek bir anda kuru çalıştırma ve son
// çalışmaların günlüğü. Claude Code dosyaları değişmeden Claude Code'da da çalışır. Proje dosyası ekiple
// paylaşılır (depoya girer). CLI: kor hooks list|add|edit|rm|test|log.
import { FlaskConical, Pencil, Plus, RefreshCw, ScrollText, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { HookDryRun, HookFile, HookItem } from '../../../ipc-ek14.ts'
import { errorMessage } from '../../api.ts'
import { hooksApi } from '../../ek14-api.ts'
import type { KorPanelProps } from '../registry.ts'
import { emptyForm, formOf, itemOf, summary, type Action, type HookForm } from './hook-form.ts'

const fileLabel = (f: HookFile) => `${f.format === 'kor' ? 'Kor hooks.toml' : 'Claude Code'} · ${f.scope === 'user' ? 'kullanıcı' : f.scope === 'local' ? 'yerel' : 'proje'}`

function DryRunView({ d }: { d: HookDryRun }) {
  return (
    <div className="ks-entry" role="status">
      <p className={d.matches ? 'ks-ok' : 'ks-warn'}>{d.matches ? 'Bu anda çalışırdı.' : `Çalışmazdı: ${d.skipped.join('; ')}`}</p>
      {d.korEvents.length > 0 && <p className="ks-entry-meta muted">Kor olayları: {d.korEvents.join(', ')}</p>}
      {d.executed && (
        <>
          <p className="ks-entry-meta">{d.ms} ms</p>
          {d.blocked && <p className="error-text">Engelledi: <span translate="no">{d.blocked}</span></p>}
          {d.context.map((c, i) => <pre key={`c${i}`} className="ks-code" translate="no">{c}</pre>)}
          {d.notices.map((n, i) => <p key={`n${i}`} className="ks-warn" translate="no">{n}</p>)}
        </>
      )}
    </div>
  )
}

function Editor({ files, events, initial, onDone }: { files: HookFile[]; events: { kor: string[]; claude: string[] }; initial: { fileIndex: number; form: HookForm; replace: { event: string; id: string } | null }; onDone: (saved: boolean) => void }) {
  const uid = useId()
  const [fileIndex, setFileIndex] = useState(initial.fileIndex)
  const [f, setF] = useState<HookForm>(initial.form)
  const [err, setErr] = useState<string | null>(null)
  const [dry, setDry] = useState<HookDryRun | null>(null)
  const [sample, setSample] = useState({ tool: 'Edit', file: 'src/main.rs', model: '' })
  const file = files[fileIndex]!
  const evs = file.format === 'kor' ? events.kor : events.claude
  const set = <K extends keyof HookForm>(k: K, v: HookForm[K]) => setF((p) => ({ ...p, [k]: v }))
  const item = itemOf(f, file)
  const save = (e: FormEvent) => {
    e.preventDefault()
    if (typeof item === 'string') return setErr(item)
    hooksApi.save(fileIndex, item, initial.replace).then(() => onDone(true)).catch((x: unknown) => setErr(errorMessage(x)))
  }
  const test = (execute: boolean) => {
    if (typeof item === 'string') return setErr(item)
    setErr(null)
    hooksApi.test(fileIndex, item, { tool: sample.tool || null, file: sample.file || null, model: sample.model || null }, execute).then(setDry).catch((x: unknown) => setErr(errorMessage(x)))
  }
  return (
    <form className="ks-form" onSubmit={save} aria-label="Hook düzenle">
      <label className="ks-field" htmlFor={`${uid}-file`}>Dosya</label>
      <select id={`${uid}-file`} className="g7-select" value={fileIndex} onChange={(e) => setFileIndex(Number(e.target.value))} disabled={initial.replace !== null}>
        {files.map((x, i) => <option key={x.path} value={i}>{fileLabel(x)}</option>)}
      </select>
      <label className="ks-field" htmlFor={`${uid}-event`}>Olay</label>
      <select id={`${uid}-event`} className="g7-select" value={f.event} onChange={(e) => set('event', e.target.value)}>
        {!evs.includes(f.event) && <option value={f.event}>{f.event || 'Seçin…'}</option>}
        {evs.map((ev) => <option key={ev} value={ev}>{ev}</option>)}
      </select>
      <fieldset className="ks-inline" aria-label="Eylem">
        {(['command', 'mcp', 'notify'] as Action[]).map((a) => (
          <label key={a} className="ks-check">
            <input type="radio" name={`${uid}-action`} checked={f.action === a} onChange={() => set('action', a)} disabled={file.format === 'claude' && a !== 'command'} />
            {a === 'command' ? 'Komut / betik' : a === 'mcp' ? 'MCP çağrısı' : 'Bildirim'}
          </label>
        ))}
      </fieldset>
      {f.action === 'command' && <input className="text-input" aria-label="Komut" value={f.command} onChange={(e) => set('command', e.target.value)} placeholder="cargo fmt --check" translate="no" />}
      {f.action === 'mcp' && (
        <>
          <input className="text-input" aria-label="MCP aracı (sunucu/araç)" value={f.mcp} onChange={(e) => set('mcp', e.target.value)} placeholder="linter/check" translate="no" />
          <textarea className="text-input ks-textarea" aria-label="Argümanlar (JSON)" value={f.args} onChange={(e) => set('args', e.target.value)} placeholder='{ "strict": true }' translate="no" />
        </>
      )}
      {f.action === 'notify' && <input className="text-input" aria-label="Bildirim metni" value={f.notify} onChange={(e) => set('notify', e.target.value)} placeholder="Görev bitti" />}
      <details className="ks-more" open={Boolean(f.matcher || f.files || f.branch || f.model)}>
        <summary>Koşullar</summary>
        <div className="ks-form">
          <input className="text-input" aria-label="Araç deseni (matcher)" value={f.matcher} onChange={(e) => set('matcher', e.target.value)} placeholder="Araç: Edit|Write" translate="no" />
          <input className="text-input" aria-label="Dosya desenleri" value={f.files} onChange={(e) => set('files', e.target.value)} placeholder="Dosya: src/**/*.rs, tests/**" translate="no" disabled={file.format === 'claude'} />
          <input className="text-input" aria-label="Dal deseni" value={f.branch} onChange={(e) => set('branch', e.target.value)} placeholder="Dal: main" translate="no" disabled={file.format === 'claude'} />
          <input className="text-input" aria-label="Model deseni" value={f.model} onChange={(e) => set('model', e.target.value)} placeholder="Model: anthropic/*" translate="no" disabled={file.format === 'claude'} />
          <input className="text-input" aria-label="Zaman aşımı (saniye)" value={f.timeout} onChange={(e) => set('timeout', e.target.value)} placeholder="Zaman aşımı (sn): 60" inputMode="numeric" />
          <label className="ks-check"><input type="checkbox" checked={f.disabled} onChange={(e) => set('disabled', e.target.checked)} disabled={file.format === 'claude'} /> Kapalı (dosyada kalsın, çalışmasın)</label>
        </div>
      </details>
      <details className="ks-more">
        <summary>Kuru çalıştırma</summary>
        <div className="ks-form">
          <div className="ks-inline">
            <input className="text-input" aria-label="Örnek araç" value={sample.tool} onChange={(e) => setSample({ ...sample, tool: e.target.value })} placeholder="Araç" translate="no" />
            <input className="text-input" aria-label="Örnek dosya" value={sample.file} onChange={(e) => setSample({ ...sample, file: e.target.value })} placeholder="Dosya" translate="no" />
            <input className="text-input" aria-label="Örnek model" value={sample.model} onChange={(e) => setSample({ ...sample, model: e.target.value })} placeholder="Model" translate="no" />
          </div>
          <div className="ks-actions">
            <button type="button" className="btn" onClick={() => test(false)} data-kor-cmd="hooks.test"><FlaskConical size={14} aria-hidden="true" /> Koşulları dene</button>
            <button type="button" className="btn" onClick={() => test(true)} data-kor-cmd="hooks.test" title="Komutu ya da MCP çağrısını örnek veriyle gerçekten çalıştırır">Gerçekten çalıştır</button>
          </div>
          {dry && <DryRunView d={dry} />}
        </div>
      </details>
      {err && <p className="error-text" role="alert">{err}</p>}
      <div className="ks-actions">
        <button type="submit" className="btn primary" data-kor-cmd="hooks.save">Kaydet</button>
        <button type="button" className="btn ghost" onClick={() => onDone(false)}>Vazgeç</button>
      </div>
    </form>
  )
}

export function HooksPanel(_: KorPanelProps) {
  const files = useSWR('hooks:files', () => hooksApi.files(), { revalidateOnFocus: false })
  const events = useSWR('hooks:events', () => hooksApi.events(), { revalidateOnFocus: false })
  const list = useSWR('hooks:list', () => hooksApi.list(), { revalidateOnFocus: false })
  const [log, setLog] = useState<Awaited<ReturnType<typeof hooksApi.log>> | null>(null)
  const [editing, setEditing] = useState<{ fileIndex: number; form: HookForm; replace: { event: string; id: string } | null } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const fs = files.data ?? []
  const indexOf = (it: HookItem) => fs.findIndex((f) => f.path === it.file)
  const refresh = () => {
    void files.mutate()
    void list.mutate()
  }
  if (editing && files.data && events.data) {
    return (
      <section className="ks" aria-label="Hook editörü">
        <h2 className="ks-title">{editing.replace ? 'Hook düzenle' : 'Hook ekle'}</h2>
        <Editor files={files.data} events={events.data} initial={editing} onDone={(saved) => { setEditing(null); if (saved) refresh() }} />
      </section>
    )
  }
  const projectKor = Math.max(0, fs.findIndex((f) => f.format === 'kor' && f.scope === 'project'))
  return (
    <section className="ks" aria-label="Hook'lar">
      <div className="kg-card-head">
        <h2 className="ks-title">Hook'lar</h2>
        <button type="button" className="action-btn" onClick={refresh} aria-label="Hook listesini yenile" title="Yenile" data-kor-cmd="hooks.list"><RefreshCw size={16} aria-hidden="true" /></button>
        <button type="button" className="action-btn" disabled={!files.data || !events.data} onClick={() => setEditing({ fileIndex: projectKor, form: emptyForm(events.data?.kor[0] ?? 'pre-edit'), replace: null })} aria-label="Hook ekle" title="Hook ekle" data-kor-cmd="hooks.events">
          <Plus size={16} aria-hidden="true" />
        </button>
      </div>
      {err && <p className="error-text" role="alert">{err}</p>}
      {list.error ? (
        <p className="error-text" role="alert">{errorMessage(list.error)}</p>
      ) : !list.data || !files.data ? (
        <p className="muted" role="status" data-kor-cmd="hooks.files">Hook'lar okunuyor…</p>
      ) : (
        <>
          {list.data.warnings.map((w, i) => <p key={i} className="ks-warn" translate="no">{w}</p>)}
          {fs.map((file, fi) => {
            const items = list.data!.items.filter((it) => it.file === file.path)
            if (items.length === 0) return null
            return (
              <div key={file.path} className="ks-form">
                <h3 className="ks-label" title={file.path}>{fileLabel(file)}</h3>
                <ul className="ks-list">
                  {items.map((it) => (
                    <li key={`${it.event}:${it.id}`} className="ks-row">
                      <span className="ks-chip" translate="no">{it.event}</span>
                      <code translate="no" title={summary(it)}>{summary(it)}</code>
                      {it.disabled && <span className="ks-badge">kapalı</span>}
                      <button type="button" className="action-btn" onClick={() => setEditing({ fileIndex: indexOf(it), form: formOf(it), replace: { event: it.event, id: it.id } })} aria-label="Hook'u düzenle" title="Düzenle">
                        <Pencil size={14} aria-hidden="true" />
                      </button>
                      <button type="button" className="action-btn reject" onClick={() => hooksApi.remove(fi, it.event, it.id).then(refresh).catch((x: unknown) => setErr(errorMessage(x)))} aria-label="Hook'u sil" title="Sil" data-kor-cmd="hooks.remove">
                        <Trash2 size={14} aria-hidden="true" />
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )
          })}
          {list.data.items.length === 0 && (
            <div className="ks-empty">
              <p>Hook yok.</p>
              <p className="ks-note">Hook'lar düzenlemeden, komuttan, commit'ten ya da görev bitiminden önce/sonra komut, MCP aracı ya da bildirim çalıştırır. Proje dosyası (<code>.kor/hooks.toml</code>) depoyla ekibe paylaşılır.</p>
            </div>
          )}
        </>
      )}
      <div className="kg-card-head">
        <h3 className="ks-title">Günlük</h3>
        <button type="button" className="action-btn" onClick={() => hooksApi.log(50).then(setLog).catch((x: unknown) => setErr(errorMessage(x)))} aria-label="Son hook çalışmalarını göster" title="Günlük" data-kor-cmd="hooks.log">
          <ScrollText size={16} aria-hidden="true" />
        </button>
      </div>
      {log && (log.length === 0 ? <p className="ks-empty">Henüz çalışan hook yok.</p> : (
        <ul className="ks-list">
          {log.map((r, i) => (
            <li key={i} className="ks-row">
              <span className={`ks-chip${r.result === 'ok' ? '' : ' bad'}`}>{r.result}</span>
              <span className="ks-chip" translate="no">{r.event}</span>
              <code translate="no" title={r.message ?? r.command}>{r.command}</code>
              <span className="ks-time">{r.ms} ms</span>
            </li>
          ))}
        </ul>
      ))}
    </section>
  )
}
