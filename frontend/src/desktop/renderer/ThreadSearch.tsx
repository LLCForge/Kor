// [g3] Sohbet içinde Ctrl+F: eşleşmeler CSS Custom Highlight API ile boyanır (DOM'a dokunulmaz, React ile çakışmaz).
import { ChevronDown, ChevronUp, X } from 'lucide-react'
import { useEffect, useRef, useState, type KeyboardEvent, type RefObject } from 'react'
import { findMatches } from './search-utils.ts'

const MAX_MATCHES = 1000
const HIDDEN = '[hidden], [aria-hidden="true"], .collapse:not(.open), script, style'

function collectRanges(root: HTMLElement, query: string): Range[] {
  const ranges: Range[] = []
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node && ranges.length < MAX_MATCHES; node = walker.nextNode()) {
    const text = node.nodeValue ?? ''
    if (text.length < query.length || node.parentElement?.closest(HIDDEN)) continue
    for (const start of findMatches(text, query, MAX_MATCHES - ranges.length)) {
      const range = document.createRange()
      range.setStart(node, start)
      range.setEnd(node, start + query.length)
      ranges.push(range)
    }
  }
  return ranges
}

const supported = typeof CSS !== 'undefined' && 'highlights' in CSS

function paint(all: Range[], current: Range | undefined): void {
  if (!supported) return
  CSS.highlights.set('g3-search', new Highlight(...all))
  if (current) CSS.highlights.set('g3-search-current', new Highlight(current))
  else CSS.highlights.delete('g3-search-current')
}

function clearPaint(): void {
  if (!supported) return
  CSS.highlights.delete('g3-search')
  CSS.highlights.delete('g3-search-current')
}

interface ThreadSearchProps {
  /** Aranacak içerik (mesaj listesi) */
  containerRef: RefObject<HTMLElement | null>
  /** İçerik değişince eşleşmeler yeniden hesaplanır */
  version: unknown
  onClose: () => void
}

export function ThreadSearch({ containerRef, version, onClose }: ThreadSearchProps) {
  const inputRef = useRef<HTMLInputElement>(null)
  const [query, setQuery] = useState('')
  const [ranges, setRanges] = useState<Range[]>([])
  const [index, setIndex] = useState(0)

  useEffect(() => {
    inputRef.current?.focus()
    return clearPaint
  }, [])

  // Akış sürerken her parçada yeniden taramamak için kısa gecikme.
  useEffect(() => {
    const timer = setTimeout(() => {
      const root = containerRef.current
      const next = root && query.trim() ? collectRanges(root, query) : []
      setRanges(next)
      setIndex((i) => (i < next.length ? i : 0))
    }, 120)
    return () => clearTimeout(timer)
  }, [query, version, containerRef])

  const current = ranges[index]
  useEffect(() => {
    paint(ranges, current)
    current?.startContainer.parentElement?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [ranges, current])

  const step = (delta: number) => {
    if (ranges.length === 0) return
    setIndex((i) => (i + delta + ranges.length) % ranges.length)
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      step(e.shiftKey ? -1 : 1)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      onClose()
    }
  }

  const counter = query.trim() ? (ranges.length === 0 ? 'Sonuç yok' : `${index + 1}/${ranges.length}${ranges.length >= MAX_MATCHES ? '+' : ''}`) : ''

  return (
    <div className="g3-search pop-in" role="search">
      <label htmlFor="g3-search-input" className="sr-only">
        Sohbette ara
      </label>
      <input
        id="g3-search-input"
        ref={inputRef}
        className="g3-search-input"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="Sohbette ara"
        maxLength={200}
        autoComplete="off"
        spellCheck={false}
      />
      <span className="g3-search-count" aria-live="polite">
        {counter}
      </span>
      <button type="button" className="action-btn" onClick={() => step(-1)} disabled={ranges.length === 0} aria-label="Önceki eşleşme" title="Önceki (Shift+Enter)">
        <ChevronUp size={16} aria-hidden="true" />
      </button>
      <button type="button" className="action-btn" onClick={() => step(1)} disabled={ranges.length === 0} aria-label="Sonraki eşleşme" title="Sonraki (Enter)">
        <ChevronDown size={16} aria-hidden="true" />
      </button>
      <button type="button" className="action-btn" onClick={onClose} aria-label="Aramayı kapat" title="Kapat (Esc)">
        <X size={16} aria-hidden="true" />
      </button>
    </div>
  )
}
