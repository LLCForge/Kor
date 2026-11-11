import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'
import { usePresence } from './use-presence.ts'

/**
 * Açılır menüler için: dışarı tıklayınca ve Esc ile kapanır. Kapanırken menü `--kor-dur-menu` boyunca takılı kalır
 * (`open` true döner) ve sarmalayıcıya `data-closing` verilir; CSS çıkış animasyonunu oynatır (§23.1).
 */
export function usePopover<T extends HTMLElement>(): {
  open: boolean
  setOpen: (open: boolean | ((prev: boolean) => boolean)) => void
  ref: RefObject<T | null>
} {
  const [open, setOpen] = useState(false)
  const ref = useRef<T>(null)
  const shown = usePresence(open)

  useLayoutEffect(() => {
    ref.current?.toggleAttribute('data-closing', shown && !open)
  }, [shown, open])

  useEffect(() => {
    if (!open) return
    const onPointer = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('pointerdown', onPointer)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', onPointer)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  return { open: shown, setOpen, ref }
}
