import { useLayoutEffect, useState } from 'react'
import { motionMs } from './motion.ts'

/**
 * Kapanış animasyonu (§23.1: menü 200 ms, panel 300 ms): `open` false olunca öğe süre bitene kadar takılı kalır.
 * Döner: çizilsin mi. `shown && !open` iken çağıran kapanış sınıfını (`data-closing`) verir. Hareket kapalıysa
 * (süre 0) hemen kaldırılır; kapanırken yeniden açılırsa zamanlayıcı iptal olur.
 */
export function usePresence(open: boolean, kind: 'menu' | 'panel' = 'menu'): boolean {
  const [shown, setShown] = useState(open)
  useLayoutEffect(() => {
    if (open) {
      setShown(true)
      return
    }
    const ms = motionMs(kind)
    if (ms === 0) {
      setShown(false)
      return
    }
    const t = window.setTimeout(() => setShown(false), ms)
    return () => window.clearTimeout(t)
  }, [open, kind])
  return open || shown
}
