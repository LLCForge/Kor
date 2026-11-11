// [g3] LaTeX: KaTeX doğrudan DOM'a çizer. HTML dizgisi üretip ayrıştırmak (rehype-katex) CSP'ye takılan
// satır içi style özniteliklerini her çizimde yeniden ayrıştırıyordu; katex.render ise stilleri CSSOM ile atar.
import type katexApi from 'katex'
import { useLayoutEffect, useRef } from 'react'
import { loadRichBundle } from './rich-lazy.ts'

// [g16] KaTeX ilk açılış paketinde değil (242): ilk matematik ifadesinde katex.js tembel yüklenir (katex-entry.ts).
type Katex = typeof katexApi
const readyKatex = () => (window as { __g16Katex?: Katex }).__g16Katex

export function MathView({ tex, display }: { tex: string; display: boolean }) {
  const ref = useRef<HTMLSpanElement>(null)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const draw = (katex: Katex) => {
      try {
        katex.render(tex, el, { displayMode: display, throwOnError: false, strict: false, trust: false, maxExpand: 1000 })
      } catch (err) {
        // throwOnError: false iken yalnızca beklenmeyen hatalar buraya düşer; ham ifade gösterilir.
        console.warn('LaTeX çizilemedi:', err)
        el.textContent = tex
      }
    }
    const katex = readyKatex()
    if (katex) {
      draw(katex)
      return
    }
    // Paket gelene kadar ham ifade görünür.
    el.textContent = tex
    let cancelled = false
    loadRichBundle<Katex>('./katex.js', '__g16Katex')
      .then((loaded) => {
        if (!cancelled) draw(loaded)
      })
      .catch((err: unknown) => console.warn('KaTeX yüklenemedi:', err))
    return () => {
      cancelled = true
    }
  }, [tex, display])

  return <span ref={ref} className={display ? 'g3-math-display' : 'g3-math-inline'} />
}
