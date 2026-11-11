import { cloneElement, useEffect, useId, useLayoutEffect, useRef, useState, type FocusEvent, type PointerEvent, type ReactElement } from 'react'
import { TOOLTIP_DELAY_MS } from './logic.ts'

interface TriggerProps {
  'aria-describedby'?: string
  onPointerEnter?: (e: PointerEvent<HTMLElement>) => void
  onPointerLeave?: (e: PointerEvent<HTMLElement>) => void
  onFocus?: (e: FocusEvent<HTMLElement>) => void
  onBlur?: (e: FocusEvent<HTMLElement>) => void
}

export interface TooltipProps {
  text: string
  children: ReactElement<TriggerProps>
  side?: 'top' | 'bottom'
  /** Hikâyeler ve görsel regresyon için: gecikmesiz açık başlar. */
  defaultOpen?: boolean
}

/**
 * 400 ms gecikmeli tooltip (§23.1): yalnız ikonlu her kontrol `aria-label` + bu tooltip ile gelir.
 * Fare ve klavye odağıyla açılır, Esc ile kapanır; tetikleyiciye `aria-describedby` bağlanır.
 */
export function Tooltip({ text, children, side = 'top', defaultOpen = false }: TooltipProps) {
  const id = useId()
  const [open, setOpen] = useState(defaultOpen)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const clear = () => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
  }
  const show = () => {
    clear()
    timer.current = setTimeout(() => setOpen(true), TOOLTIP_DELAY_MS)
  }
  const hide = () => {
    clear()
    setOpen(false)
  }
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') hide()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open])
  useEffect(() => clear, [])
  // Pencere kenarına taşmasın: yatayda içeri kaydırılır, üstte/altta yer yoksa öbür tarafa açılır.
  const tipRef = useRef<HTMLSpanElement>(null)
  const [fit, setFit] = useState<{ x: number; flip: boolean }>({ x: 0, flip: false })
  useLayoutEffect(() => {
    const el = tipRef.current
    const anchor = el?.parentElement
    if (!open || !el || !anchor) return
    const edge = 8
    const r = el.getBoundingClientRect()
    const a = anchor.getBoundingClientRect()
    const dx = r.left < edge ? edge - r.left : r.right > window.innerWidth - edge ? window.innerWidth - edge - r.right : 0
    const flip = side === 'top' ? a.top < r.height + edge : a.bottom + r.height + edge > window.innerHeight
    setFit((f) => (dx === 0 && f.flip === flip ? f : { x: f.x + dx, flip }))
  }, [open, text, side])
  const shownSide = fit.flip ? (side === 'top' ? 'bottom' : 'top') : side
  return (
    <span className="kor-tooltip-anchor" data-kor-tooltip="">
      {cloneElement(children, {
        'aria-describedby': open ? id : undefined,
        onPointerEnter: show,
        onPointerLeave: hide,
        onFocus: show,
        onBlur: hide,
      })}
      {open && (
        <span ref={tipRef} role="tooltip" id={id} className={`kor-tooltip ${shownSide}`} style={fit.x ? { translate: `${fit.x}px 0` } : undefined}>
          {text}
        </span>
      )}
    </span>
  )
}
