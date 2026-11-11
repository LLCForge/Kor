// Graf işlemleri: dışa aktarma (biçim seçici → klasörde göster), git sürümüne göre fark, etki analizi sonucu.
import { Download, FolderOpen, GitCompare, X } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { ExportFormat, ExportReport, GraphDiff, NodeRef } from '../../../ipc-graph.ts'
import { errorMessage } from '../../api.ts'
import { graphApi } from './bridge.ts'
import { KIND_LABEL, EDGE_LABEL } from './model.ts'

const FORMATS: ReadonlyArray<{ id: ExportFormat; label: string }> = [
  { id: 'html', label: 'HTML görüntüleyici' },
  { id: 'json', label: 'JSON (Kor şeması)' },
  { id: 'graphify', label: 'graphify JSON' },
  { id: 'mermaid', label: 'Mermaid' },
  { id: 'obsidian', label: 'Obsidian kasası' },
]

const LIST_MAX = 20

export function ExportTool() {
  const [format, setFormat] = useState<ExportFormat>('html')
  const [busy, setBusy] = useState(false)
  const [report, setReport] = useState<ExportReport | null>(null)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      setReport(await graphApi.export(format))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="kg-tool">
      <div className="kg-row">
        <label className="kg-field">
          <span>Biçim</span>
          <select value={format} onChange={(e) => setFormat(e.target.value as ExportFormat)}>
            {FORMATS.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
        </label>
        <button type="button" className="btn small" onClick={() => void run()} disabled={busy} data-kor-cmd="graph.export">
          <Download size={14} aria-hidden="true" />
          {busy ? 'Aktarılıyor…' : 'Dışa aktar'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {report && (
        <div className="kg-result" role="status">
          <span className="kg-path" translate="no" title={report.path}>{report.path}</span>
          <span className="muted">
            {`${report.files_written} dosya yazıldı`}
            {report.nodes_omitted > 0 && ` · ${report.nodes_omitted} düğüm sınır nedeniyle dışarıda`}
          </span>
          <button
            type="button"
            className="btn small"
            data-kor-cmd="graph.reveal"
            onClick={() => void graphApi.reveal(report.path).catch((err: unknown) => setError(errorMessage(err)))}
          >
            <FolderOpen size={14} aria-hidden="true" />
            Klasörde göster
          </button>
        </div>
      )}
    </div>
  )
}

export function DiffTool({ onSelect }: { onSelect: (id: string) => void }) {
  const [rev, setRev] = useState('HEAD~5')
  const [busy, setBusy] = useState(false)
  const [diff, setDiff] = useState<GraphDiff | null>(null)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const r = rev.trim()
    if (!r) return
    setBusy(true)
    setError(null)
    try {
      setDiff(await graphApi.diff(r))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="kg-tool">
      <form className="kg-row" onSubmit={(e) => void submit(e)}>
        <label className="kg-field">
          <span>Git sürümü</span>
          <input value={rev} onChange={(e) => setRev(e.target.value)} placeholder="HEAD~5" spellCheck={false} maxLength={200} translate="no" />
        </label>
        <button type="submit" className="btn small" disabled={busy || !rev.trim()} data-kor-cmd="graph.diff">
          <GitCompare size={14} aria-hidden="true" />
          {busy ? 'Karşılaştırılıyor…' : 'Karşılaştır'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {diff && (
        <div className="kg-result" role="status">
          <span className="muted">{`${diff.rev} sürümünden bu yana: +${diff.added_nodes.length} / −${diff.removed_nodes.length} düğüm, +${diff.added_edges.length} / −${diff.removed_edges.length} kenar`}</span>
          <NodeList title="Eklenen düğümler" nodes={diff.added_nodes} onSelect={onSelect} />
          <NodeList title="Silinen düğümler" nodes={diff.removed_nodes} />
          <EdgeList title="Eklenen kenarlar" edges={diff.added_edges} />
          <EdgeList title="Silinen kenarlar" edges={diff.removed_edges} />
        </div>
      )}
    </div>
  )
}

export function ImpactResult({ id, onSelect, onClose }: { id: string; onSelect: (id: string) => void; onClose: () => void }) {
  const { data, error, isLoading } = useSWR(['graph:impact', id], () => graphApi.impact(id, 3), { revalidateOnFocus: false })
  return (
    <section className="kg-tool kg-impact" aria-label="Etki analizi" data-kor-cmd="graph.impact">
      <header className="kg-card-head">
        <span className="kg-card-title">
          Etki analizi <span className="kg-card-name" translate="no">{id}</span>
        </span>
        <button type="button" className="action-btn" onClick={onClose} aria-label="Etki analizini kapat" title="Kapat">
          <X size={14} aria-hidden="true" />
        </button>
      </header>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading || !data ? (
        <p className="muted">Etkilenenler hesaplanıyor…</p>
      ) : data.affected.length === 0 ? (
        <p className="muted">Bu düğüme bağımlı başka düğüm yok.</p>
      ) : (
        <>
          <span className="muted">
            {`${data.affected.length} düğüm, ${data.files.length} dosya etkileniyor`}
            {data.truncated && ' (liste kısaltıldı)'}
          </span>
          <ul className="kg-plain">
            {data.affected.slice(0, LIST_MAX).map((a) => (
              <li key={a.node.id}>
                <button type="button" className="kg-link" onClick={() => onSelect(a.node.id)} title={a.node.file ?? a.node.id}>
                  <span translate="no">{a.node.name}</span>
                  <span className="kg-link-meta">{`${a.depth}. derece · ${EDGE_LABEL[a.via.kind]}`}</span>
                </button>
              </li>
            ))}
          </ul>
          {data.affected.length > LIST_MAX && <p className="muted kg-more">{`… ${data.affected.length - LIST_MAX} öğe daha`}</p>}
        </>
      )}
    </section>
  )
}

function NodeList({ title, nodes, onSelect }: { title: string; nodes: NodeRef[]; onSelect?: (id: string) => void }) {
  if (nodes.length === 0) return null
  return (
    <div className="kg-links">
      <h4>{`${title} (${nodes.length})`}</h4>
      <ul>
        {nodes.slice(0, LIST_MAX).map((n) => (
          <li key={n.id}>
            {onSelect ? (
              <button type="button" className="kg-link" onClick={() => onSelect(n.id)} title={n.file ?? n.id}>
                <span translate="no">{n.name}</span>
                <span className="kg-link-meta">{KIND_LABEL[n.kind]}</span>
              </button>
            ) : (
              <span className="kg-link" title={n.file ?? n.id}>
                <span translate="no">{n.name}</span>
                <span className="kg-link-meta">{KIND_LABEL[n.kind]}</span>
              </span>
            )}
          </li>
        ))}
      </ul>
      {nodes.length > LIST_MAX && <p className="muted kg-more">{`… ${nodes.length - LIST_MAX} öğe daha`}</p>}
    </div>
  )
}

function EdgeList({ title, edges }: { title: string; edges: GraphDiff['added_edges'] }) {
  if (edges.length === 0) return null
  return (
    <div className="kg-links">
      <h4>{`${title} (${edges.length})`}</h4>
      <ul>
        {edges.slice(0, LIST_MAX).map((e) => (
          <li key={`${e.source}>${e.target}>${e.kind}`} className="kg-edge" translate="no">
            {`${e.source} → ${e.target} (${e.kind})`}
          </li>
        ))}
      </ul>
      {edges.length > LIST_MAX && <p className="muted kg-more">{`… ${edges.length - LIST_MAX} öğe daha`}</p>}
    </div>
  )
}
