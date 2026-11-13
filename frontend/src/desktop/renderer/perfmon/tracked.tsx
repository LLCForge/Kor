// §32.2: `memo` + gereksiz çizim sayacı (rerenders.ts). Release'de birebir `memo`.
import { memo, useEffect, useRef, type FunctionComponent, type NamedExoticComponent } from 'react'
import { RERENDER_TRACKING, noteRender } from './rerenders.ts'

export function trackedMemo<P extends object>(name: string, Component: FunctionComponent<P>): NamedExoticComponent<P> {
  if (!RERENDER_TRACKING) return memo(Component)
  function Tracked(props: P) {
    const prev = useRef<P | null>(null)
    // İşlenen (commit edilen) çizimler sayılır: StrictMode'un çift çağrısı ve atılan çizimler değil.
    useEffect(() => {
      noteRender(name, prev.current, props)
      prev.current = props
    })
    // Aynı bileşenin içinde düz çağrı: kancalar her çizimde aynı sırada.
    return Component(props)
  }
  Tracked.displayName = name
  return memo(Tracked)
}
