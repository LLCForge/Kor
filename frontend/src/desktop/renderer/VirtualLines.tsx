import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'

const LINE_HEIGHT = 20
const OVERSCAN = 20

interface VirtualLinesProps {
  text: string
  /** 1 tabanlı; verilirse o satıra kaydırılır ve vurgulanır */
  highlightLine?: number | null
  /** Aynı satıra yeniden gitmek için değişen sayaç */
  scrollKey?: number
  label: string
}

/** Büyük dosyalar için satır bazlı pencereleme: yalnızca görünen satırlar DOM'da. */
export function VirtualLines({ text, highlightLine, scrollKey, label }: VirtualLinesProps) {
  const lines = useMemo(() => text.split(/\r?\n/), [text])
  const maxLen = useMemo(() => lines.reduce((m, l) => Math.max(m, l.length), 0), [lines])
  const ref = useRef<HTMLDivElement>(null)
  const [range, setRange] = useState({ start: 0, end: 100 })

  const update = useCallback(() => {
    const el = ref.current
    if (!el) return
    const start = Math.max(0, Math.floor(el.scrollTop / LINE_HEIGHT) - OVERSCAN)
    const end = Math.min(lines.length, Math.ceil((el.scrollTop + el.clientHeight) / LINE_HEIGHT) + OVERSCAN)
    setRange((prev) => (prev.start === start && prev.end === end ? prev : { start, end }))
  }, [lines.length])

  useLayoutEffect(update, [update])

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => observer.disconnect()
  }, [update])

  useLayoutEffect(() => {
    const el = ref.current
    if (!el || !highlightLine) return
    el.scrollTop = Math.max(0, (highlightLine - 1) * LINE_HEIGHT - el.clientHeight / 3)
    update()
  }, [highlightLine, scrollKey, update])

  const gutter = String(lines.length).length
  return (
    <div ref={ref} className="g4-vlines" onScroll={update} tabIndex={0} role="region" aria-label={label}>
      <div className="g4-vlines-spacer" style={{ height: lines.length * LINE_HEIGHT, minWidth: `${maxLen + gutter + 4}ch` }}>
        <div className="g4-vlines-window" style={{ transform: `translateY(${range.start * LINE_HEIGHT}px)` }}>
          {lines.slice(range.start, range.end).map((line, i) => {
            const no = range.start + i + 1
            return (
              <div key={no} className={`g4-vline${no === highlightLine ? ' highlight' : ''}`} aria-current={no === highlightLine ? 'location' : undefined}>
                <span className="g4-vline-no" style={{ width: `${gutter + 2}ch` }} aria-hidden="true">{no}</span>
                <span className="g4-vline-text">{line}</span>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
