// [g3] Mermaid dışı, SVG üreten diyagram dilleri için ortak kart: Graphviz (```dot/```graphviz), nomnoml
//      (```nomnoml), WaveDrom (```wavedrom). Ağır kütüphane tembel paketten yüklenir; çıktı SVG metni
//      doğrudan ana DOM'a değil, SandboxedPreview kind="svg" ile (otomatik DOMPurify, ağsız iframe) gösterilir.
import { Code, GitFork, Network, Workflow } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { parseDiagramError } from './diagram-utils.ts'
import { loadRichBundle } from './rich-lazy.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { SandboxedPreview } from './SandboxedPreview.tsx'

/** Tembel paketin açtığı diyagram çizici yüzeyi (senkron ya da asenkron SVG üretir). */
interface DiagramApi {
  render(code: string): string | Promise<string>
}

/** Akış sürerken her parçada yeniden çizmemek için kod durulunca çizilir (Mermaid ile aynı gecikme). */
const SETTLE_MS = 400

/** SVG metninden yaklaşık görüntü yüksekliği (iframe'in içeriği kırpmaması için); sınırlandırılır. */
function estimateSvgHeight(svg: string): number {
  const m = /<svg\b[^>]*\bheight="([\d.]+)(pt|px)?"/i.exec(svg)
  if (!m) return 340
  const value = Number(m[1]) * (m[2] === 'pt' ? 1.3333 : 1)
  return Math.min(640, Math.max(160, Math.round(value) + 24))
}

interface SvgDiagramBlockProps extends RichBlockProps {
  label: string
  icon: ReactNode
  bundle: { src: string; globalKey: string }
  /**
   * SVG gösterim kipi. 'svg' (varsayılan): DOMPurify ile temizlenir + düğüm sayısı sınırı (öznitelik tabanlı
   * çizicilerde: Graphviz, nomnoml). 'html': WaveDrom gibi <style> bloğuna bağlı çıktılar için; temizlenmeden
   * ama ağsız, betiksiz, opak origin'li sandbox iframe içinde izole edilir (stil korunur, kod çalışamaz).
   */
  kind?: 'svg' | 'html'
}

export function SvgDiagramBlock({ code, label, icon, bundle, kind = 'svg' }: SvgDiagramBlockProps) {
  const [svg, setSvg] = useState<string | null>(null)
  const [error, setError] = useState<{ line: number | null; message: string } | null>(null)
  const [showSource, setShowSource] = useState(false)

  useEffect(() => {
    let cancelled = false
    setSvg(null)
    setError(null)
    const timer = setTimeout(() => {
      loadRichBundle<DiagramApi>(bundle.src, bundle.globalKey)
        .then((api) => api.render(code))
        .then(
          (out) => {
            if (!cancelled) setSvg(out)
          },
          (err: unknown) => {
            if (cancelled) return
            setError(parseDiagramError(err instanceof Error ? err.message : String(err)))
          },
        )
    }, SETTLE_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [code, bundle.src, bundle.globalKey])

  return (
    <div className="code-block g3-mermaid g3-svg-diagram">
      <div className="code-head">
        <span className="g3-code-lang">
          {icon} {label}
        </span>
        <div className="g3-code-tools">
          <button
            type="button"
            className={`code-copy${showSource ? ' g3-on' : ''}`}
            onClick={() => setShowSource((s) => !s)}
            aria-pressed={showSource}
            aria-label="Kaynak kodu göster"
            title="Kaynak kodu göster"
          >
            <Code size={15} aria-hidden="true" />
          </button>
          <CopyButton className="code-copy" label={`${label} kaynağını kopyala`} getText={() => code} />
        </div>
      </div>
      {!svg && !error && <p className="g3-mermaid-note">Diyagram çiziliyor…</p>}
      {error && (
        <p className="g3-mermaid-note error-text" role="alert">
          Diyagram çizilemedi{error.line ? ` (satır ${error.line})` : ''}: {error.message}
        </p>
      )}
      {svg && <SandboxedPreview content={svg} kind={kind} title={`${label} diyagramı`} height={estimateSvgHeight(svg)} />}
      {(showSource || error) && <pre className="g3-mermaid-source">{code}</pre>}
    </div>
  )
}

// feature 16: Graphviz (DOT). feature 19: nomnoml. feature 18: WaveDrom.
const GRAPHVIZ = { src: './graphviz.js', globalKey: '__g3Graphviz' }
const NOMNOML = { src: './nomnoml.js', globalKey: '__g3Nomnoml' }
const WAVEDROM = { src: './wavedrom.js', globalKey: '__g3Wavedrom' }

function GraphvizBlock(props: RichBlockProps) {
  return <SvgDiagramBlock {...props} label="Graphviz" icon={<GitFork size={13} aria-hidden="true" />} bundle={GRAPHVIZ} />
}
function NomnomlBlock(props: RichBlockProps) {
  return <SvgDiagramBlock {...props} label="nomnoml UML" icon={<Network size={13} aria-hidden="true" />} bundle={NOMNOML} />
}
function WaveDromBlock(props: RichBlockProps) {
  // WaveDrom <style> bloğuna dayanır; temizleyici onu çıkarır, bu yüzden izole html kipinde gösterilir.
  return <SvgDiagramBlock {...props} label="WaveDrom" icon={<Workflow size={13} aria-hidden="true" />} bundle={WAVEDROM} kind="html" />
}

registerRichBlock('dot', GraphvizBlock)
registerRichBlock('graphviz', GraphvizBlock)
registerRichBlock('nomnoml', NomnomlBlock)
registerRichBlock('wavedrom', WaveDromBlock)
