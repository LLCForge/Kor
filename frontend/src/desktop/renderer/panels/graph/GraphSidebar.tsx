// Grafın sağ kenar paneli (graphify görünümü): çizim teması, düğüm arama, seçili düğümün bilgisi, renkleri ve
// sayılarıyla topluluk listesi (aç/kapa, tümünü seç), altta görünen düğüm/kenar/topluluk sayıları.
import { Search } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import type { Snapshot } from '../../../ipc-graph.ts'
import { uiLocale } from '../../ui-locale.ts'
import { KIND_LABEL, findNodes } from './model.ts'

export type GraphTheme = 'light' | 'dark' | 'system'

const THEMES: ReadonlyArray<{ id: GraphTheme; label: string }> = [
  { id: 'light', label: 'Açık' },
  { id: 'dark', label: 'Koyu' },
  { id: 'system', label: 'Sistem' },
]

interface Props {
  snapshot: Snapshot
  slots: Int32Array
  hidden: ReadonlySet<number>
  onHidden: (next: ReadonlySet<number>) => void
  selected: number | null
  onSelect: (index: number) => void
  theme: GraphTheme
  onTheme: (t: GraphTheme) => void
  visibleNodes: number
  visibleEdges: number
}

export function GraphSidebar({ snapshot, slots, hidden, onHidden, selected, onSelect, theme, onTheme, visibleNodes, visibleEdges }: Props) {
  const id = useId()
  const [q, setQ] = useState('')
  const matches = useMemo(() => findNodes(snapshot.nodes, q, 8), [snapshot, q])
  // Topluluk başına renk: ilk üyesinin palet yeri.
  const slotOf = useMemo(() => {
    const m = new Map<number, number>()
    snapshot.nodes.forEach((n, i) => {
      if (!m.has(n.community)) m.set(n.community, slots[i] ?? 0)
    })
    return m
  }, [snapshot, slots])
  const communities = useMemo(() => [...snapshot.communities].sort((a, b) => b.size - a.size), [snapshot])
  const allOn = hidden.size === 0
  const node = selected !== null ? snapshot.nodes[selected] : undefined
  const community = node ? snapshot.communities.find((c) => c.id === node.community) : undefined
  const fmt = (n: number) => n.toLocaleString(uiLocale())

  return (
    <aside className="kg-side" aria-label="Graf ayrıntıları">
      <div className="kg-side-themes" role="group" aria-label="Çizim teması">
        {THEMES.map((t) => (
          <button key={t.id} type="button" className={`kg-side-theme${theme === t.id ? ' active' : ''}`} aria-pressed={theme === t.id} onClick={() => onTheme(t.id)}>
            {t.label}
          </button>
        ))}
      </div>

      <label className="cdx-search kg-side-search" htmlFor={`${id}-q`}>
        <Search size={13} aria-hidden="true" />
        <input id={`${id}-q`} type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Düğüm ara…" aria-label="Düğüm ara" spellCheck={false} maxLength={200} translate="no" />
      </label>
      {q.trim() && (
        <ul className="kg-plain kg-side-hits" aria-label="Eşleşen düğümler">
          {matches.length === 0 ? (
            <li className="muted kg-side-note">Eşleşen düğüm yok.</li>
          ) : (
            matches.map((i) => {
              const n = snapshot.nodes[i]!
              return (
                <li key={n.id}>
                  <button type="button" className="kg-link" onClick={() => onSelect(i)} title={n.file ?? n.id}>
                    <span className="kg-side-dot" data-slot={slots[i]} aria-hidden="true" />
                    <span translate="no">{n.name}</span>
                    <span className="kg-link-meta">{KIND_LABEL[n.kind]}</span>
                  </button>
                </li>
              )
            })
          )}
        </ul>
      )}

      <section className="kg-side-block" aria-labelledby={`${id}-info`}>
        <h3 id={`${id}-info`} className="kg-side-h">Düğüm bilgisi</h3>
        {node ? (
          <dl className="kg-side-info">
            <dt>Ad</dt>
            <dd translate="no">{node.name}</dd>
            <dt>Tür</dt>
            <dd>{KIND_LABEL[node.kind]}</dd>
            {node.file && (
              <>
                <dt>Dosya</dt>
                <dd translate="no">{node.line ? `${node.file}:${node.line}` : node.file}</dd>
              </>
            )}
            <dt>Topluluk</dt>
            <dd translate="no">{community?.label ?? String(node.community)}</dd>
            <dt>Bağlantı</dt>
            <dd>{fmt(node.degree)}</dd>
          </dl>
        ) : (
          <p className="muted kg-side-note">İncelemek için bir düğüme tıkla.</p>
        )}
      </section>

      <section className="kg-side-block kg-side-communities" aria-labelledby={`${id}-com`}>
        <h3 id={`${id}-com`} className="kg-side-h">Topluluklar</h3>
        <label className="kg-check">
          <input type="checkbox" checked={allOn} onChange={() => onHidden(allOn ? new Set(communities.map((c) => c.id)) : new Set())} />
          Tümünü seç
        </label>
        <ul className="kg-plain kg-side-list">
          {communities.map((c) => (
            <li key={c.id}>
              <label className="kg-check kg-side-row">
                <input
                  type="checkbox"
                  checked={!hidden.has(c.id)}
                  onChange={(e) => {
                    const next = new Set(hidden)
                    if (e.target.checked) next.delete(c.id)
                    else next.add(c.id)
                    onHidden(next)
                  }}
                />
                <span className="kg-side-dot" data-slot={slotOf.get(c.id) ?? 0} aria-hidden="true" />
                <span className="kg-side-name" translate="no" title={c.label}>{c.label}</span>
                <span className="kg-side-count">{fmt(c.size)}</span>
              </label>
            </li>
          ))}
        </ul>
      </section>

      <p className="muted kg-side-foot" role="status">
        {`${fmt(visibleNodes)} düğüm · ${fmt(visibleEdges)} kenar · ${fmt(communities.length)} topluluk`}
      </p>
    </aside>
  )
}
