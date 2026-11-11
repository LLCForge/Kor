// §27.14 diyagram paneli (ADR 0063): dokümanlardaki işaretli Mermaid/PlantUML/D2 blokları, grafa
// göre durumları (güncel / eskimiş / elle düzenlenmiş), düğüm-kenar özeti ve yeni üretilen metin;
// senkronla, elle düzenleneni zorla, yeni blok ekle. CLI: kor diagram sync|watch|check|list|add.
import { Plus, RefreshCw, Wand2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { DiagramChange } from '../../../ipc-ek14.ts'
import { errorMessage } from '../../api.ts'
import { diagramApi } from '../../ek14-api.ts'
import type { KorPanelProps } from '../registry.ts'
import { parseParams, statusLabel } from './diagram-form.ts'

const KINDS = ['architecture', 'dependency', 'sequence', 'er', 'deploy'] as const
const FORMATS = ['mermaid', 'plantuml', 'd2'] as const

function Block({ c }: { c: DiagramChange }) {
  const n = c.counts
  return (
    <li className="ks-entry">
      <div className="ks-entry-head">
        <code translate="no">{c.file}:{c.line}</code>
        {c.kind && <span className="ks-chip">{c.kind}</span>}
        <span className={`ks-badge${c.status === 'up_to_date' ? ' on' : ''}`}>{statusLabel(c.status)}</span>
        {c.status === 'stale' && <span className="ks-label">+{n.nodes_added} −{n.nodes_removed} düğüm, +{n.edges_added} −{n.edges_removed} kenar</span>}
      </div>
      {c.error && <p className="ks-warn">{c.error}</p>}
      {c.ascii.length > 0 && (
        <details className="ks-more">
          <summary>Özet ({c.ascii.length})</summary>
          <pre className="ks-code" translate="no">{c.ascii.join('\n')}</pre>
        </details>
      )}
      {c.text && (
        <details className="ks-more">
          <summary>Üretilen metin</summary>
          <pre className="ks-code" translate="no">{c.text}</pre>
        </details>
      )}
    </li>
  )
}

export function DiagramsPanel(_: KorPanelProps) {
  const uid = useId()
  const { data, error, isValidating, mutate } = useSWR('diagram:list', () => diagramApi.list(), { revalidateOnFocus: false })
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const [file, setFile] = useState('docs/architecture.md')
  const [kind, setKind] = useState<string>('architecture')
  const [format, setFormat] = useState<string>('mermaid')
  const [params, setParams] = useState('')
  const run = (p: Promise<{ written: string[]; changes: DiagramChange[]; summary: string[] }>) => {
    setBusy(true)
    setNote(null)
    p.then((r) => {
      void mutate(r.changes, { revalidate: false })
      setNote({ text: r.written.length === 0 ? 'Bütün diyagramlar güncel.' : `Diyagram güncellendi: ${r.summary.join('; ') || r.written.join(', ')}`, ok: true })
    })
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
      .finally(() => setBusy(false))
  }
  const add = (e: FormEvent) => {
    e.preventDefault()
    run(diagramApi.add(file.trim(), kind, format, parseParams(params)))
  }
  const edited = data?.some((c) => c.status === 'edited') ?? false
  return (
    <section className="ks" aria-label="Diyagramlar">
      <div className="kg-card-head">
        <h2 className="ks-title">Diyagramlar</h2>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Diyagram durumunu yenile" title="Yenile" data-kor-cmd="diagram.list" disabled={isValidating}>
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      <div className="ks-actions">
        <button type="button" className="btn primary" disabled={busy} onClick={() => run(diagramApi.sync(false))} data-kor-cmd="diagram.sync">
          <Wand2 size={14} aria-hidden="true" /> Senkronla
        </button>
        {edited && (
          <button type="button" className="btn danger" disabled={busy} onClick={() => run(diagramApi.sync(true))} data-kor-cmd="diagram.sync" title="Elle düzenlenmiş üretilmiş bölgelerin üzerine yazar (kor:keep sonrası korunur)">
            Elle düzenlenenleri de yenile
          </button>
        )}
      </div>
      {note && <p className={note.ok ? 'ks-ok' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">Graf güncelleniyor, diyagramlar okunuyor…</p>
      ) : data.length === 0 ? (
        <div className="ks-empty">
          <p>İşaretli diyagram yok.</p>
          <p className="ks-note">Aşağıdan bir doküman ve tür seçerek ekleyin; Kor işaretli bölgeyi kod değiştikçe yeniden üretir, <code>kor:keep</code> satırından sonrası sizindir. CI için: <code>kor diagram check</code>.</p>
        </div>
      ) : (
        <ul className="ks-list" data-kor-cmd="diagram.list">{data.map((c) => <Block key={`${c.file}:${c.line}`} c={c} />)}</ul>
      )}
      <details className="ks-more">
        <summary><Plus size={12} aria-hidden="true" /> Diyagram ekle</summary>
        <form className="ks-form" onSubmit={add}>
          <label className="ks-field" htmlFor={`${uid}-file`}>Doküman dosyası (.md, .mmd, .puml, .d2)</label>
          <input id={`${uid}-file`} className="text-input" value={file} onChange={(e) => setFile(e.target.value)} translate="no" />
          <div className="ks-inline">
            <label className="sr-only" htmlFor={`${uid}-kind`}>Tür</label>
            <select id={`${uid}-kind`} className="g7-select" value={kind} onChange={(e) => setKind(e.target.value)}>{KINDS.map((k) => <option key={k} value={k}>{k}</option>)}</select>
            <label className="sr-only" htmlFor={`${uid}-format`}>Biçim</label>
            <select id={`${uid}-format`} className="g7-select" value={format} onChange={(e) => setFormat(e.target.value)}>{FORMATS.map((f) => <option key={f} value={f}>{f}</option>)}</select>
          </div>
          <label className="ks-field" htmlFor={`${uid}-params`}>Parametreler (ör. <code>scope=src depth=2 from=main</code>)</label>
          <input id={`${uid}-params`} className="text-input" value={params} onChange={(e) => setParams(e.target.value)} translate="no" />
          <button type="submit" className="btn" disabled={busy || !file.trim()} data-kor-cmd="diagram.add">Ekle ve üret</button>
        </form>
      </details>
    </section>
  )
}
