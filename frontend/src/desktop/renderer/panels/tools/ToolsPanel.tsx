// Araçlar ve niyetler paneli (§26.2, §30, ADR 0056): PATH'teki komut satırı araçları (sürümleriyle),
// tipli araç tanımları (ekle, paylaş, çek), bir programın yardım özeti, ve doğal dil yönlendirmesi
// (hangi cümle hangi komuta gider, öğretilen cümleler). CLI: kor tool … / kor intents ….
import { BookOpen, Download, Plus, RefreshCw, Search, Trash2, Upload, Wand2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { IntentDecision, ToolHelp, ToolsApi, TypedTool } from '../../../ipc-intent.ts'
import { api, errorMessage } from '../../api.ts'
import { headline } from '../../intent/format.ts'
import { intentApi } from '../../intent/IntentProposal.tsx'
import type { KorPanelProps } from '../registry.ts'

const toolsApi = (api as unknown as { tools: ToolsApi }).tools

type Note = { text: string; ok: boolean } | null

function Status({ note }: { note: Note }) {
  if (!note) return null
  return <p className={note.ok ? 'kt-ok' : 'kt-error'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>
}

function Catalog() {
  const uid = useId()
  const [refresh, setRefresh] = useState(false)
  const { data, error, isLoading, mutate } = useSWR(['tools:list', refresh], () => toolsApi.list(refresh))
  const [all, setAll] = useState(false)
  if (isLoading) return <p className="muted">PATH taranıyor…</p>
  if (error) return <p className="kt-error" role="alert">{errorMessage(error)}</p>
  if (!data) return null
  const rows = data.catalog.tools.filter((t) => all || t.known || t.description !== '')
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <div className="kg-card-head">
        <h3 id={`${uid}-h`} className="kg-card-title">Bu bilgisayardaki araçlar</h3>
        <button type="button" className="action-btn" onClick={() => { setRefresh(true); void mutate() }} aria-label="PATH'i yeniden tara" title="Yeniden tara" data-kor-cmd="tools.list">
          <RefreshCw size={13} aria-hidden="true" />
        </button>
      </div>
      <p className="kt-small muted">
        {data.catalog.tools.length} program, {rows.length} tanesi açıklamalı. ⇄ ağ kullanır.{' '}
        <label><input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} /> Tümü</label>
      </p>
      {rows.length === 0 ? (
        <p className="muted">PATH'te tanınan bir program yok.</p>
      ) : (
        <ul className="kt-list">
          {rows.slice(0, 300).map((t) => (
            <li key={t.name}>
              <code translate="no">{t.name}</code> <span className="muted">{t.version ?? ''}</span> {t.description}{t.network ? ' ⇄' : ''}
            </li>
          ))}
        </ul>
      )}
      {data.typed.length > 0 && <TypedList typed={data.typed} />}
      {data.errors.map((e) => <p key={e} className="kt-error" translate="no">{e}</p>)}
      {!data.trusted && <p className="kt-small muted">Projeye güvenene kadar bu projenin .kor/tools/ klasörü yok sayılır.</p>}
    </section>
  )
}

function TypedList({ typed }: { typed: TypedTool[] }) {
  return (
    <>
      <h4 className="kt-h">Tipli araçlar</h4>
      <ul className="kt-list">
        {typed.map((t) => (
          <li key={t.name}>
            <code translate="no">{t.name}</code> <span className="muted">{t.origin} · {t.safety}</span> <span translate="no">{t.command.join(' ')}</span>
          </li>
        ))}
      </ul>
    </>
  )
}

function Describe() {
  const uid = useId()
  const [name, setName] = useState('')
  const [help, setHelp] = useState<ToolHelp | null>(null)
  const [hits, setHits] = useState<string[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<Note>(null)
  const describe = (model: boolean) => {
    setBusy(true)
    setNote(null)
    toolsApi
      .describe(name.trim(), false, model)
      .then((h) => { setHelp(h); setHits(null) })
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
      .finally(() => setBusy(false))
  }
  const search = (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setNote(null)
    toolsApi
      .search(name.trim())
      .then((r) => { setHits(r.hits); setHelp(null) })
      .catch((err: unknown) => setNote({ text: errorMessage(err), ok: false }))
      .finally(() => setBusy(false))
  }
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h3 id={`${uid}-h`} className="kg-card-title">Ara ya da yardımını oku</h3>
      <form className="kt-form" onSubmit={search}>
        <input className="text-input" aria-label="Program adı ya da iş" placeholder="ffmpeg · video dönüştür" value={name} onChange={(e) => setName(e.target.value)} translate="no" />
        <button type="submit" className="btn" disabled={busy || name.trim() === ''} data-kor-cmd="tools.search"><Search size={13} aria-hidden="true" /> Ara</button>
        <button type="button" className="btn" disabled={busy || name.trim() === ''} onClick={() => describe(false)} data-kor-cmd="tools.describe"><BookOpen size={13} aria-hidden="true" /> Yardım</button>
        <button type="button" className="btn" disabled={busy || name.trim() === ''} onClick={() => describe(true)} data-kor-cmd="tools.describe" title="Model özeti ekler (ücretli olabilir)"><Wand2 size={13} aria-hidden="true" /> Model özeti</button>
      </form>
      <Status note={note} />
      {hits && (hits.length === 0 ? <p className="muted">Eşleşen kurulu araç yok.</p> : <ul className="kt-list">{hits.map((h) => <li key={h} translate="no">{h}</li>)}</ul>)}
      {help && (
        <div className="kt-help" translate="no">
          {help.model_summary && <pre>{help.model_summary}</pre>}
          {!help.model_summary && help.summary && <p>{help.summary}</p>}
          {help.usage && <p><code>{help.usage}</code></p>}
          {help.subcommands.length > 0 && <ul className="kt-list">{help.subcommands.map(([c, d]) => <li key={c}><code>{c}</code> {d}</li>)}</ul>}
          {help.flags.length > 0 && <ul className="kt-list">{help.flags.map(([f, d]) => <li key={f}><code>{f}</code> {d}</li>)}</ul>}
        </div>
      )}
    </section>
  )
}

function AddTool() {
  const uid = useId()
  const [form, setForm] = useState({ name: '', command: '', safety: 'write' as TypedTool['safety'], description: '', user: false })
  const [note, setNote] = useState<Note>(null)
  const [busy, setBusy] = useState(false)
  const submit = (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    toolsApi
      .add(form)
      .then((r) => setNote({ text: `Tanımlandı: ${r.path}`, ok: true }))
      .catch((err: unknown) => setNote({ text: errorMessage(err), ok: false }))
      .finally(() => setBusy(false))
  }
  const team = (fn: () => Promise<string>) => {
    setBusy(true)
    fn().then((t) => setNote({ text: t, ok: true })).catch((err: unknown) => setNote({ text: errorMessage(err), ok: false })).finally(() => setBusy(false))
  }
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h3 id={`${uid}-h`} className="kg-card-title">Tipli araç tanımla</h3>
      <form className="kt-form" onSubmit={submit}>
        <label className="kt-field"><span>Ad</span><input className="text-input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="thumbnail" translate="no" required /></label>
        <label className="kt-field kt-grow"><span>Komut şablonu</span><input className="text-input" value={form.command} onChange={(e) => setForm({ ...form, command: e.target.value })} placeholder="ffmpeg -i {input} -frames:v 1 {output}" translate="no" required /></label>
        <label className="kt-field"><span>Güvenlik</span>
          <select className="text-input" value={form.safety} onChange={(e) => setForm({ ...form, safety: e.target.value as TypedTool['safety'] })}>
            <option value="read">salt okunur</option>
            <option value="write">yazar</option>
            <option value="network">ağa çıkar</option>
            <option value="destructive">yıkıcı (her zaman sorar)</option>
          </select>
        </label>
        <label className="kt-field kt-grow"><span>Açıklama</span><input className="text-input" value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} /></label>
        <label className="kt-check"><input type="checkbox" checked={form.user} onChange={(e) => setForm({ ...form, user: e.target.checked })} /> Tüm projeler için</label>
        <button type="submit" className="btn primary" disabled={busy} data-kor-cmd="tools.add"><Plus size={13} aria-hidden="true" /> Ekle</button>
      </form>
      <div className="kt-row">
        <button type="button" className="btn" disabled={busy} data-kor-cmd="tools.share" onClick={() => team(async () => `${(await toolsApi.share()).count} tanım takım sunucusuna gönderildi.`)}><Upload size={13} aria-hidden="true" /> Takımla paylaş</button>
        <button type="button" className="btn" disabled={busy} data-kor-cmd="tools.pull" onClick={() => team(async () => { const r = await toolsApi.pull(); return `${r.names.length} takım aracı alındı${r.rejected.length ? `, ${r.rejected.length} reddedildi` : ''}.` })}><Download size={13} aria-hidden="true" /> Takımdan çek</button>
      </div>
      <Status note={note} />
    </section>
  )
}

function describeDecision(d: IntentDecision): string {
  switch (d.kind) {
    case 'agent':
      return 'Komut değil: ajana gider.'
    case 'run':
    case 'confirm':
    case 'missing':
      return `${headline(d)} → ${d.plan.line}`
    case 'choose':
      return `${headline(d)} ${d.options.map((o) => o.line).join(' · ')}`
    case 'pipeline':
      return `${headline(d)}: ${d.steps.map((s) => (s.type === 'command' ? s.value.line : `ajan: ${s.value}`)).join(' → ')}`
    case 'schedule':
      return `${headline(d)} ${d.prompt}`
  }
}

function Intents() {
  const uid = useId()
  const { data, error, mutate } = useSWR('intent:list', () => intentApi.list())
  const [text, setText] = useState('')
  const [command, setCommand] = useState('')
  const [result, setResult] = useState<string | null>(null)
  const [note, setNote] = useState<Note>(null)
  const test = (e: FormEvent) => {
    e.preventDefault()
    intentApi.route(text).then((d) => setResult(describeDecision(d))).catch((err: unknown) => setResult(errorMessage(err)))
  }
  const learn = () => {
    intentApi
      .learn(text, command.startsWith('kor') ? command : `kor ${command}`)
      .then(() => { setNote({ text: 'Öğrenildi.', ok: true }); void mutate() })
      .catch((err: unknown) => setNote({ text: errorMessage(err), ok: false }))
  }
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h3 id={`${uid}-h`} className="kg-card-title">Doğal dil komutları</h3>
      <form className="kt-form" onSubmit={test}>
        <input className="text-input kt-grow" aria-label="Cümle" placeholder="maliyetimi göster" value={text} onChange={(e) => setText(e.target.value)} />
        <button type="submit" className="btn" disabled={text.trim() === ''} data-kor-cmd="intent.route">Dene</button>
        <input className="text-input kt-grow" aria-label="Komut" placeholder="kor cost" value={command} onChange={(e) => setCommand(e.target.value)} translate="no" />
        <button type="button" className="btn" disabled={text.trim() === '' || command.trim() === ''} onClick={learn} data-kor-cmd="intent.learn">Öğret</button>
      </form>
      {result && <p className="kt-small" translate="no">{result}</p>}
      <Status note={note} />
      {error && <p className="kt-error" role="alert">{errorMessage(error)}</p>}
      {data && data.learned.length > 0 && (
        <>
          <h4 className="kt-h">Öğrettiğiniz cümleler</h4>
          <ul className="kt-list">
            {data.learned.map((p) => (
              <li key={p.text}>
                <span translate="no">"{p.text}" → {p.command}</span>
                <button type="button" className="action-btn" aria-label="Cümleyi unut" title="Unut" data-kor-cmd="intent.forget" onClick={() => void intentApi.forget(p.text).then(() => mutate())}>
                  <Trash2 size={12} aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      {data && (
        <details>
          <summary className="kt-small" data-kor-cmd="intent.list">{data.intents.length} komut serbest metinle çağrılabilir</summary>
          <ul className="kt-list">
            {data.intents.map((i) => (
              <li key={i.command}>
                <code translate="no">{i.command}</code> <span className="muted">{i.run === 'auto' ? 'hemen çalışır' : 'önce sorar'}</span>{' '}
                <span translate="no">{i.phrases.slice(0, 3).map((p) => `"${p.text}"`).join(' · ')}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  )
}

export function ToolsPanel(_props: KorPanelProps) {
  return (
    <div className="kt">
      <Intents />
      <Describe />
      <AddTool />
      <Catalog />
    </div>
  )
}
