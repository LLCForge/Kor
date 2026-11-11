// [g3] feature 27: Büyük diyagram için konum haritası. Kaydırılabilir kabın (host) gölge kökündeki SVG'nin
//      ölçeklenmiş bir kopyasını gösterir; görünür alanı bir dikdörtgenle işaretler ve tıklayınca oraya kaydırır.
//      Kopya kendi gölge köküne konur (kimlik çakışması olmaz) ve kaynağın biçem sayfaları paylaşılır.
import { useEffect, useRef, type MouseEvent } from 'react'

const WIDTH = 150

export function DiagramMinimap({ host }: { host: HTMLElement }) {
  const boxRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLDivElement>(null)
  const viewportRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const box = boxRef.current
    const canvas = canvasRef.current
    const sourceRoot = host.shadowRoot
    const svg = sourceRoot?.querySelector('svg')
    if (!box || !canvas || !sourceRoot || !svg) return

    const root = canvas.shadowRoot ?? canvas.attachShadow({ mode: 'open' })
    // Kaynak diyagramın biçem sayfaları paylaşılır (mermaid renkleri korunur); .style JS ile uygulanır (CSP güvenli).
    root.adoptedStyleSheets = sourceRoot.adoptedStyleSheets
    const clone = svg.cloneNode(true) as SVGSVGElement
    clone.removeAttribute('width')
    clone.removeAttribute('height')
    clone.style.width = '100%'
    clone.style.height = 'auto'
    clone.style.display = 'block'
    root.replaceChildren(clone)

    const sync = () => {
      const vp = viewportRef.current
      if (!vp) return
      const sw = host.scrollWidth || 1
      const sh = host.scrollHeight || 1
      const boxW = box.clientWidth
      const boxH = box.clientHeight
      vp.style.left = `${(host.scrollLeft / sw) * boxW}px`
      vp.style.top = `${(host.scrollTop / sh) * boxH}px`
      vp.style.width = `${Math.min(boxW, (host.clientWidth / sw) * boxW)}px`
      vp.style.height = `${Math.min(boxH, (host.clientHeight / sh) * boxH)}px`
    }
    sync()
    host.addEventListener('scroll', sync, { passive: true })
    const observer = new ResizeObserver(sync)
    observer.observe(host)
    return () => {
      host.removeEventListener('scroll', sync)
      observer.disconnect()
    }
  }, [host])

  const scrollTo = (e: MouseEvent<HTMLDivElement>) => {
    const box = boxRef.current
    if (!box) return
    const rect = box.getBoundingClientRect()
    const fx = (e.clientX - rect.left) / rect.width
    const fy = (e.clientY - rect.top) / rect.height
    host.scrollTo({
      left: fx * host.scrollWidth - host.clientWidth / 2,
      top: fy * host.scrollHeight - host.clientHeight / 2,
      behavior: 'smooth',
    })
  }

  return (
    <div
      ref={boxRef}
      className="g3-dgm-minimap"
      style={{ width: WIDTH }}
      onClick={scrollTo}
      role="presentation"
      aria-hidden="true"
      title="Konum haritası — tıklayarak kaydır"
    >
      <div ref={canvasRef} className="g3-dgm-canvas" />
      <div ref={viewportRef} className="g3-dgm-viewport" />
    </div>
  )
}
