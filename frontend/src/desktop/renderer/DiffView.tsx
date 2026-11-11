import { diffLines } from 'diff'
import type hljsApi from 'highlight.js/lib/common'
import { ChevronsUpDown, MessageSquare, Plus, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { loadRichBundle } from './rich-lazy.ts'
import type { FileChange } from './chat-state.ts'
import { splitRows, unifiedRows, type SplitCell, type SplitRow, type UnifiedRow, type WordSeg } from './diff-align.ts'
import { foldContext } from './diff-fold.ts'
import { addReviewComment, removeReviewComment, useReviewComments, type ReviewComment } from './review-comments.ts'

export type DiffMode = 'unified' | 'split'
export const DIFF_MODES = ['unified', 'split'] as const

const SIGN = { add: '+', del: '-', ctx: ' ' } as const

// Dosya uzantısından sözdizimi dili (highlight.js/lib/common kapsamı).
const LANG_BY_EXT: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  json: 'json', py: 'python', rb: 'ruby', go: 'go', rs: 'rust', java: 'java',
  c: 'c', h: 'c', cpp: 'cpp', cc: 'cpp', hpp: 'cpp', cs: 'csharp', php: 'php',
  css: 'css', scss: 'scss', less: 'less', html: 'xml', xml: 'xml', svg: 'xml',
  md: 'markdown', sh: 'bash', bash: 'bash', zsh: 'bash', ps1: 'powershell',
  yml: 'yaml', yaml: 'yaml', sql: 'sql', swift: 'swift', kt: 'kotlin', toml: 'ini', ini: 'ini',
}
// [perf] highlight.js tembel yüklenir (dist/renderer/hljs.js); gelene kadar düz metin.
type Hljs = typeof hljsApi
let hljs: Hljs | null = (window as { __korHljs?: Hljs }).__korHljs ?? null
const hljsListeners = new Set<() => void>()
function ensureHljs(): void {
  if (hljs) return
  void loadRichBundle<Hljs>('./hljs.js', '__korHljs')
    .then((api) => {
      hljs = api
      for (const l of hljsListeners) l()
    })
    .catch((err: unknown) => console.warn('Sözdizimi vurgusu yüklenemedi:', err))
}
function useHljsReady(): boolean {
  const [ready, setReady] = useState(hljs !== null)
  useEffect(() => {
    if (hljs) return
    const l = () => setReady(true)
    hljsListeners.add(l)
    ensureHljs()
    return () => { hljsListeners.delete(l) }
  }, [])
  return ready
}
function langOf(path: string): string | null {
  const ext = path.split('.').pop()?.toLowerCase() ?? ''
  const lang = LANG_BY_EXT[ext]
  return lang && hljs?.getLanguage(lang) ? lang : null
}
function escapeHtml(s: string): string {
  return s.replace(/[&<>]/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : '&gt;'))
}
// Satır bazlı vurgu (diff +/- arka planı korunur; çok satırlı yapılar nadiren hafif kayabilir — okunabilirlik önceliği).
function highlightLine(text: string, lang: string | null): { __html: string } {
  if (!lang || !text || !hljs) return { __html: escapeHtml(text) }
  try {
    return { __html: hljs.highlight(text, { language: lang }).value }
  } catch {
    return { __html: escapeHtml(text) }
  }
}

/** Satır metni: kelime parçaları varsa değişen parçalar koyu vurgulu (cdesktop), yoksa tek parça sözdizimi renkli. */
function LineText({ text, segs, lang }: { text: string; segs?: WordSeg[]; lang: string | null }) {
  if (!segs) return <span className="hljs" dangerouslySetInnerHTML={highlightLine(text, lang)} />
  return (
    <span className="hljs">
      {segs.map((s, i) =>
        s.changed ? (
          <mark key={i} className="diff-word" dangerouslySetInnerHTML={highlightLine(s.text, lang)} />
        ) : (
          <span key={i} dangerouslySetInnerHTML={highlightLine(s.text, lang)} />
        ),
      )}
    </span>
  )
}

/** cdesktop satır içi yorum editörü: turuncu kutu, "İnceleme yorumu ekle" / "İptal". */
function CommentEditor({ onSubmit, onCancel }: { onSubmit: (text: string) => void; onCancel: () => void }) {
  const [text, setText] = useState('')
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (text.trim()) onSubmit(text)
  }
  return (
    <form className="cdx-review-editor" onSubmit={submit}>
      <textarea
        className="cdx-review-input"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="Bu satır hakkında…"
        aria-label="İnceleme yorumu"
        autoFocus
        rows={2}
        onKeyDown={(e) => {
          if (e.key === 'Escape') { e.preventDefault(); onCancel() }
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); if (text.trim()) onSubmit(text) }
        }}
      />
      <div className="cdx-review-actions">
        <button type="submit" className="btn small primary" disabled={!text.trim()}>İnceleme yorumu ekle</button>
        <button type="button" className="btn small cdx-review-cancel" onClick={onCancel}>İptal</button>
      </div>
    </form>
  )
}

function CommentCard({ c }: { c: ReviewComment }) {
  return (
    <div className="cdx-review-card" role="note">
      <MessageSquare size={13} aria-hidden="true" />
      <span className="cdx-review-text">{c.text}</span>
      <button type="button" className="action-btn" onClick={() => removeReviewComment(c.id)} aria-label="Yorumu sil" title="Yorumu sil">
        <Trash2 size={12} aria-hidden="true" />
      </button>
    </div>
  )
}

type LineKey = string
const keyOf = (side: 'old' | 'new', no: number) => `${side}:${no}`

function Cell({ cell, side, lang }: { cell: SplitCell | null; side: 'left' | 'right'; lang: string | null }) {
  if (!cell) return <td className={`g4-split-cell empty ${side}`} colSpan={2} />
  return (
    <>
      <td className={`g4-ln ${cell.kind}`}>{cell.no}</td>
      <td className={`g4-split-cell ${cell.kind} ${side}`}>
        <span className="diff-sign" aria-hidden="true">{SIGN[cell.kind]}</span>
        <LineText text={cell.text} segs={cell.segs} lang={lang} />
      </td>
    </>
  )
}

export function DiffView({ change, mode, reviewable = true }: { change: FileChange; mode: DiffMode; reviewable?: boolean }) {
  const parts = useMemo(() => diffLines(change.before ?? '', change.after), [change])
  const unified = useMemo(() => (mode === 'unified' ? unifiedRows(parts) : []), [parts, mode])
  const split = useMemo(() => (mode === 'split' ? splitRows(parts) : []), [parts, mode])
  const hlReady = useHljsReady()
  const lang = useMemo(() => (hlReady ? langOf(change.path) : null), [change.path, hlReady])
  const all = useReviewComments()
  const comments = useMemo(() => all.filter((c) => c.path === change.path), [all, change.path])
  const byLine = useMemo(() => {
    const m = new Map<LineKey, ReviewComment[]>()
    for (const c of comments) {
      const k = keyOf(c.side, c.line)
      m.set(k, [...(m.get(k) ?? []), c])
    }
    return m
  }, [comments])
  const [editing, setEditing] = useState<{ side: 'old' | 'new'; line: number; code: string } | null>(null)
  // Katlanan bağlam blokları (cdesktop "N unmodified lines"); anahtar = segment sırası
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(new Set())
  const unifiedSegs = useMemo(() => foldContext(unified, (r: UnifiedRow) => r.kind === 'ctx'), [unified])
  const splitSegs = useMemo(() => foldContext(split, (r: SplitRow) => r.left?.kind === 'ctx' && r.right?.kind === 'ctx'), [split])
  const foldRow = (key: number, count: number, cols?: number) => (
    <button type="button" className="cdx-fold" onClick={() => setExpanded((prev) => new Set(prev).add(key))} aria-label={`${count} değişmemiş satırı göster`} style={cols ? undefined : undefined}>
      <ChevronsUpDown size={12} aria-hidden="true" /> {count} değişmemiş satır
    </button>
  )

  const submitComment = (text: string) => {
    if (!editing) return
    addReviewComment({ path: change.path, line: editing.line, side: editing.side, code: editing.code, text })
    setEditing(null)
  }
  const addBtn = (side: 'old' | 'new', line: number, code: string) =>
    reviewable ? (
      <button type="button" className="cdx-review-add" onClick={() => setEditing({ side, line, code })} aria-label={`${line}. satıra yorum ekle`} title="İnceleme yorumu ekle">
        <Plus size={11} aria-hidden="true" />
      </button>
    ) : null
  const extras = (side: 'old' | 'new', line: number) => {
    const list = byLine.get(keyOf(side, line)) ?? []
    const isEditing = editing?.side === side && editing.line === line
    if (list.length === 0 && !isEditing) return null
    return (
      <div className="cdx-review-slot">
        {list.map((c) => <CommentCard key={c.id} c={c} />)}
        {isEditing && <CommentEditor onSubmit={submitComment} onCancel={() => setEditing(null)} />}
      </div>
    )
  }

  if (mode === 'split') {
    return (
      <div className="g4-diff-scroll">
        <table className="g4-split" aria-label={`${change.path} farkları, yan yana`}>
          <colgroup>
            <col className="g4-col-ln" />
            <col />
            <col className="g4-col-ln" />
            <col />
          </colgroup>
          <thead className="sr-only">
            <tr>
              <th scope="col">Eski satır</th>
              <th scope="col">Eski içerik</th>
              <th scope="col">Yeni satır</th>
              <th scope="col">Yeni içerik</th>
            </tr>
          </thead>
          <tbody>
            {splitSegs.flatMap((seg, si) => {
              if (seg.kind === 'fold' && !expanded.has(si)) {
                return [
                  <tr key={`fold-${si}`} className="cdx-fold-row">
                    <td colSpan={reviewable ? 5 : 4}>{foldRow(si, seg.count)}</td>
                  </tr>,
                ]
              }
              return seg.rows.map((row, ri) => {
              const i = `${si}-${ri}`
              const side: 'old' | 'new' = row.right ? 'new' : 'old'
              const no = row.right?.no ?? row.left?.no ?? 0
              const code = row.right?.text ?? row.left?.text ?? ''
              const extra = extras(side, no)
              return (
                <tr key={i} className="cdx-diff-row">
                  <Cell cell={row.left} side="left" lang={lang} />
                  <Cell cell={row.right} side="right" lang={lang} />
                  {reviewable && (
                    <td className="cdx-review-gutter">
                      {addBtn(side, no, code)}
                      {extra && <div className="cdx-review-float">{extra}</div>}
                    </td>
                  )}
                </tr>
              )
              })
            })}
          </tbody>
        </table>
      </div>
    )
  }

  // Yeni dosya: her satır "eklendi"; tamamı yeşil zemin yerine sözdizimi renkli kod gibi gösterilir (yalnız + işareti yeşil).
  const isNew = change.before === null
  return (
    <div className="g4-diff-scroll">
      <pre className={`diff g4-unified${isNew ? ' g4-new-file' : ''}`} aria-label={isNew ? `${change.path} (yeni dosya)` : `${change.path} farkları`}>
        {unifiedSegs.flatMap((seg, si) => {
          if (seg.kind === 'fold' && !expanded.has(si)) return [<span key={`fold-${si}`} className="cdx-fold-line">{foldRow(si, seg.count)}{'\n'}</span>]
          return seg.rows.map((row, ri) => {
          const i = `${si}-${ri}`
          const side: 'old' | 'new' = row.newNo !== null ? 'new' : 'old'
          const no = row.newNo ?? row.oldNo ?? 0
          const extra = extras(side, no)
          return (
            <span key={i} className="cdx-diff-line-wrap">
              <span className={`diff-line ${row.kind}`}>
                <span className="g4-ln" aria-hidden="true">{row.oldNo ?? ''}</span>
                <span className="g4-ln" aria-hidden="true">{row.newNo ?? ''}</span>
                <span className="diff-sign" aria-hidden="true">{SIGN[row.kind]}</span>
                {addBtn(side, no, row.text)}
                <LineText text={row.text} segs={row.segs} lang={lang} />
                {'\n'}
              </span>
              {extra}
            </span>
          )
          })
        })}
      </pre>
    </div>
  )
}
