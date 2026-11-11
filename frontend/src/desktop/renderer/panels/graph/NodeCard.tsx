// Seçili düğüm kartı (§7.6): kaynak satırı, topluluk, derece, çağıranlar/çağırdıkları, değişim sıklığı,
// birlikte değişme oranı ve [Ajana ver] [Açıkla] [Yol] [Etki] eylemleri.
import { BookOpen, Bot, FileCode, Route, Target, X } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { Link, NodeRef, SnapshotNode } from '../../../ipc-graph.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { insertIntoDraft } from '../../g21/quote.ts'
import { graphApi } from './bridge.ts'
import { EDGE_LABEL, KIND_LABEL, referenceFor, topCoChange } from './model.ts'

interface NodeCardProps {
  id: string
  /** Çizimde varsa anlık bilgiler (açıklama gelmeden gösterilir) */
  node: SnapshotNode | null
  onSelect: (id: string) => void
  onClose: () => void
  onPath: (id: string) => void
  onImpact: (id: string) => void
  pathPending: boolean
}

const LIST_MAX = 8

export function NodeCard({ id, node, onSelect, onClose, onPath, onImpact, pathPending }: NodeCardProps) {
  const { data, error, isLoading } = useSWR(['graph:explain', id], () => graphApi.explain(id), { revalidateOnFocus: false })
  const [detail, setDetail] = useState(false)
  const n = data?.node
  const ref: NodeRef | null = n
    ? { id: n.id, kind: n.kind, name: n.name, file: n.file, line: n.span?.[0] ?? null }
    : node
      ? { id: node.id, kind: node.kind, name: node.name, file: node.file, line: node.line }
      : null
  const co = data ? topCoChange(data) : null
  const degree = n?.degree ?? node?.degree
  const churn = n?.churn_30d ?? node?.churn

  return (
    <section className="kg-card" aria-label="Seçili düğüm" data-kor-cmd="graph.explain">
      <header className="kg-card-head">
        <span className="kg-card-title">
          <span className="kg-card-name" translate="no">{ref?.name ?? id}</span>
          {ref && <span className="badge neutral">{KIND_LABEL[ref.kind]}</span>}
        </span>
        <button type="button" className="action-btn" onClick={onClose} aria-label="Seçimi kaldır" title="Seçimi kaldır">
          <X size={14} aria-hidden="true" />
        </button>
      </header>

      {ref?.file && (
        <button type="button" className="kg-source" onClick={() => g3Actions.current?.openFile(ref.file!, ref.line ?? undefined)} title="Dosyayı önizlemede aç">
          <FileCode size={13} aria-hidden="true" />
          <span translate="no">{ref.line !== null ? `${ref.file}:${ref.line}` : ref.file}</span>
        </button>
      )}

      <dl className="kg-facts">
        <div>
          <dt>Topluluk</dt>
          <dd translate="no">{data?.community?.label ?? (node ? `#${node.community}` : '—')}</dd>
        </div>
        <div>
          <dt>Derece</dt>
          <dd>{degree ?? '—'}</dd>
        </div>
        <div>
          <dt>Son 30 gün</dt>
          <dd>{churn === undefined ? '—' : churn === 0 ? 'değişmedi' : `${churn} commit`}</dd>
        </div>
        <div>
          <dt>Birlikte değişme</dt>
          <dd>{co ? <span title={co.file} translate="no">{`%${Math.round(co.ratio * 100)} · ${co.file}`}</span> : '—'}</dd>
        </div>
      </dl>

      <div className="kg-actions">
        {ref && (
          <button type="button" className="btn small" onClick={() => insertIntoDraft(referenceFor(ref))} title="Bu düğümü mesaj kutusuna başvuru olarak ekle">
            <Bot size={14} aria-hidden="true" />
            Ajana ver
          </button>
        )}
        <button type="button" className="btn small" aria-expanded={detail} onClick={() => setDetail((d) => !d)} data-kor-cmd="graph.explain">
          <BookOpen size={14} aria-hidden="true" />
          Açıkla
        </button>
        <button type="button" className={`btn small${pathPending ? ' active' : ''}`} aria-pressed={pathPending} onClick={() => onPath(id)} data-kor-cmd="graph.path" title="İkinci bir düğüm seç; aradaki en kısa yol çizilir">
          <Route size={14} aria-hidden="true" />
          Yol
        </button>
        <button type="button" className="btn small" onClick={() => onImpact(id)} data-kor-cmd="graph.impact" title="Bu düğüm değişirse etkilenenler">
          <Target size={14} aria-hidden="true" />
          Etki
        </button>
      </div>

      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading ? (
        <p className="muted">Düğüm açıklanıyor…</p>
      ) : data === null ? (
        <p className="muted">Bu düğüm grafta artık yok (dosya değişmiş olabilir).</p>
      ) : data ? (
        <>
          <Links title="Çağıranlar" links={data.callers} onSelect={onSelect} />
          <Links title="Çağırdıkları" links={data.callees} onSelect={onSelect} />
          {detail && (
            <>
              {data.source ? (
                <pre className="kg-code" translate="no">{data.source}</pre>
              ) : (
                <p className="muted">Kaynak satırı yok.</p>
              )}
              <Links title="Birlikte değişen dosyalar" links={data.co_changes} onSelect={onSelect} />
              <Refs title="Gerekçe ve kararlar" refs={data.decisions} onSelect={onSelect} />
              <Refs title="Aynı adla eşleşen diğer düğümler" refs={data.alternatives} onSelect={onSelect} />
            </>
          )}
        </>
      ) : null}
    </section>
  )
}

function Links({ title, links, onSelect }: { title: string; links: Link[]; onSelect: (id: string) => void }) {
  if (links.length === 0) return null
  return (
    <div className="kg-links">
      <h4>{`${title} (${links.length})`}</h4>
      <ul>
        {links.slice(0, LIST_MAX).map((l) => (
          <li key={`${l.node.id}-${l.kind.kind}`}>
            <button type="button" className="kg-link" onClick={() => onSelect(l.node.id)} title={l.node.file ?? l.node.id}>
              <span translate="no">{l.node.name}</span>
              <span className="kg-link-meta">
                {EDGE_LABEL[l.kind.kind]}
                {l.kind.ratio !== undefined && ` %${Math.round(l.kind.ratio * 100)}`}
                {l.confidence.level !== 'extracted' && ` · ${l.confidence.level === 'inferred' ? 'tahmini' : 'belirsiz'}`}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {links.length > LIST_MAX && <p className="muted kg-more">{`… ${links.length - LIST_MAX} öğe daha`}</p>}
    </div>
  )
}

function Refs({ title, refs, onSelect }: { title: string; refs: NodeRef[]; onSelect: (id: string) => void }) {
  if (refs.length === 0) return null
  return (
    <div className="kg-links">
      <h4>{`${title} (${refs.length})`}</h4>
      <ul>
        {refs.slice(0, LIST_MAX).map((r) => (
          <li key={r.id}>
            <button type="button" className="kg-link" onClick={() => onSelect(r.id)} title={r.file ?? r.id}>
              <span translate="no">{r.name}</span>
              <span className="kg-link-meta">{KIND_LABEL[r.kind]}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
