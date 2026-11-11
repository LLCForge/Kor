// cdesktop "Changes" paneli: değişen dosyalar klasör ağacında, dosya satırında dil rozeti + "+N -M", üstte
// "Dosya ara…" ve tümünü daralt. Klasörler her zaman açık başlar; ağaç yalnızca bu oturumda değişen dosyaları içerir.
import { AlertTriangle, ChevronDown, ChevronRight, ChevronsDownUp, Folder, FolderOpen, Search, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { ExternalStatus } from '../ipc-g4.ts'
import { buildTree, type FilterNode } from './fuzzy.ts'
import { LangBadge } from './LangBadge.tsx'

export interface ChangeRowInfo {
  path: string
  status: 'new' | 'modified' | 'deleted'
  added: number
  removed: number
  external?: ExternalStatus
}

interface Props {
  rows: readonly ChangeRowInfo[]
  selected: string | null
  onSelect: (path: string) => void
}

const STATUS_LABEL = { new: 'Yeni', modified: 'Değişti', deleted: 'Silindi' } as const

/** Tek çocuklu klasör zincirleri birleşmez (cdesktop da birleştirmez); arama dosya yoluna göre süzer. */
export function filterChangeRows<T extends { path: string }>(rows: readonly T[], query: string): T[] {
  const q = query.trim().toLocaleLowerCase('tr')
  if (!q) return [...rows]
  return rows.filter((r) => r.path.toLocaleLowerCase('tr').includes(q))
}

function Node({ node, depth, byPath, selected, onSelect, collapseSeq }: { node: FilterNode; depth: number; byPath: ReadonlyMap<string, ChangeRowInfo>; selected: string | null; onSelect: (p: string) => void; collapseSeq: number }) {
  const [open, setOpen] = useState(true)
  const [seenSeq, setSeenSeq] = useState(collapseSeq)
  // "Tümünü daralt" sayacı değişince klasör kapanır (render sırasında state eşitleme: ek efekt gerekmez).
  if (seenSeq !== collapseSeq) {
    setSeenSeq(collapseSeq)
    setOpen(false)
  }
  const pad = { paddingLeft: 8 + depth * 14 }
  if (node.isDir) {
    return (
      <li role="treeitem" aria-expanded={open}>
        <button type="button" className="tree-row cdx-change-dir" style={pad} onClick={() => setOpen((o) => !o)} title={node.path}>
          {open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}
          {open ? <FolderOpen size={14} aria-hidden="true" /> : <Folder size={14} aria-hidden="true" />}
          <span className="tree-name">{node.name}</span>
        </button>
        {open && (
          <ul role="group" className="tree-list">
            {node.children.map((c) => (
              <Node key={`${c.isDir ? 'd' : 'f'}:${c.path}`} node={c} depth={depth + 1} byPath={byPath} selected={selected} onSelect={onSelect} collapseSeq={collapseSeq} />
            ))}
          </ul>
        )}
      </li>
    )
  }
  const info = byPath.get(node.path)
  if (!info) return null
  return (
    <li role="treeitem" aria-selected={selected === node.path}>
      <button
        type="button"
        className={`tree-row cdx-change-file${selected === node.path ? ' selected' : ''}${info.status === 'deleted' ? ' deleted' : ''}`}
        style={pad}
        onClick={() => onSelect(node.path)}
        title={node.path}
        aria-label={`${node.path}, ${STATUS_LABEL[info.status]}, ${info.added} satır eklendi, ${info.removed} satır silindi${info.external ? ', harici değişiklik var' : ''}`}
      >
        <span className="tree-chevron" aria-hidden="true" />
        <LangBadge path={node.path} size="sm" generic />
        <span className="tree-name">{node.name}</span>
        {info.external && <AlertTriangle size={12} className="g4-warn g4-shrink" aria-hidden="true" />}
        <span className="cdx-change-stats" aria-hidden="true">
          {info.added > 0 && <span className="stat-add">+{info.added}</span>}
          {info.removed > 0 && <span className="stat-del">-{info.removed}</span>}
        </span>
      </button>
    </li>
  )
}

export function ChangesTree({ rows, selected, onSelect }: Props) {
  const [query, setQuery] = useState('')
  const [collapseSeq, setCollapseSeq] = useState(0)
  const visible = useMemo(() => filterChangeRows(rows, query), [rows, query])
  const byPath = useMemo(() => new Map(visible.map((r) => [r.path, r])), [visible])
  const nodes = useMemo(() => buildTree(visible.map((r) => r.path)), [visible])
  return (
    <div className="cdx-changes">
      <div className="cdx-changes-tools">
        <label className="cdx-search">
          <Search size={13} aria-hidden="true" />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Dosya ara…" aria-label="Değişen dosyalarda ara" spellCheck={false} />
          {query && (
            <button type="button" className="action-btn" onClick={() => setQuery('')} aria-label="Aramayı temizle">
              <X size={12} aria-hidden="true" />
            </button>
          )}
        </label>
        <button type="button" className="action-btn" onClick={() => setCollapseSeq((n) => n + 1)} aria-label="Tümünü daralt" title="Tümünü daralt">
          <ChevronsDownUp size={14} aria-hidden="true" />
        </button>
      </div>
      {nodes.length === 0 ? (
        <p className="tree-status">“{query}” ile eşleşen dosya yok.</p>
      ) : (
        <ul role="tree" className="tree-list cdx-change-tree" aria-label="Değişen dosyalar">
          {nodes.map((n) => (
            <Node key={`${n.isDir ? 'd' : 'f'}:${n.path}`} node={n} depth={0} byPath={byPath} selected={selected} onSelect={onSelect} collapseSeq={collapseSeq} />
          ))}
        </ul>
      )}
    </div>
  )
}
