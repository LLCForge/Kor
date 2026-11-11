import { useLayoutEffect, useState, type CSSProperties, type RefObject } from 'react'

interface AnchorOptions {
  /** Menü tetikleyicinin sağ veya sol kenarına hizalanır. */
  align?: 'left' | 'right'
  gap?: number
  minHeight?: number
  maxHeight?: number
}

/**
 * Açılır menüyü `position: fixed` ile tetikleyiciye göre konumlar: kaydırılabilen/`overflow: hidden` kaplar
 * menüyü kırpamaz. Yerde kalmazsa yukarı açılır, yüksekliği boşluğa göre sınırlanır, yatayda pencereye sığdırılır.
 * Menü genişliği CSS'ten gelir; bu yüzden hizalama için menünün ölçülen genişliği kullanılır.
 */
export function useAnchoredMenu(
  open: boolean,
  triggerRef: RefObject<HTMLElement | null>,
  menuRef: RefObject<HTMLElement | null>,
  { align = 'right', gap = 6, minHeight = 140, maxHeight = 460 }: AnchorOptions = {},
): CSSProperties | undefined {
  const [style, setStyle] = useState<CSSProperties | undefined>(undefined)
  useLayoutEffect(() => {
    if (!open) return
    const place = () => {
      const t = triggerRef.current?.getBoundingClientRect()
      if (!t) return
      const w = menuRef.current?.offsetWidth ?? 240
      const left =
        align === 'right'
          ? Math.min(Math.max(16, t.right - w), window.innerWidth - w - 16)
          : Math.min(Math.max(16, t.left), window.innerWidth - w - 16)
      const above = t.top - 16
      const below = window.innerHeight - t.bottom - 16
      const openDown = below >= Math.min(maxHeight, minHeight) || below >= above
      const space = Math.max(minHeight, openDown ? below : above)
      setStyle({
        position: 'fixed',
        left,
        right: 'auto',
        maxHeight: Math.min(maxHeight, space),
        overflowY: 'auto',
        ...(openDown ? { top: t.bottom + gap, bottom: 'auto' } : { bottom: window.innerHeight - t.top + gap, top: 'auto' }),
      })
    }
    place()
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => {
      window.removeEventListener('resize', place)
      window.removeEventListener('scroll', place, true)
    }
  }, [open, triggerRef, menuRef, align, gap, minHeight, maxHeight])
  return style
}
