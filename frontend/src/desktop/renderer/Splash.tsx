// §16.1 açılış ekranı: uygulama çizilirken kısa süre ortada KorThinking küpü, ardından `--kor-dur-panel` içinde söner.
import { useEffect, useState } from 'react'
import { KorThinking } from './brand/brand.ts'
import { usePresence } from './use-presence.ts'

/** Küp en az bu kadar görünür (ms); sonra kapanış animasyonu başlar. */
const SPLASH_MS = 900

export function Splash() {
  const [open, setOpen] = useState(true)
  const shown = usePresence(open, 'panel')
  useEffect(() => {
    const t = window.setTimeout(() => setOpen(false), SPLASH_MS)
    return () => window.clearTimeout(t)
  }, [])
  if (!shown) return null
  return (
    <div className={`kor-splash${open ? '' : ' closing'}`} role="status" aria-label="Kor açılıyor">
      <KorThinking size={96} label="Kor açılıyor" />
    </div>
  )
}
