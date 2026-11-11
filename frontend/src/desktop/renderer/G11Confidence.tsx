// [g11] ```confidence bloğu: yanıt sonundaki güven düzeyini küçük bir rozet ve kısa gerekçe olarak gösterir.
import { parseConfidence, type ConfidenceLevel } from '../../core/intel-g11.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'

const LABELS: Record<ConfidenceLevel, string> = { high: 'Yüksek', medium: 'Orta', low: 'Düşük' }

function ConfidenceBlock({ code }: RichBlockProps) {
  const parsed = parseConfidence(code)
  // Akış sürerken blok yarımdır; biçime uymayan blok ham hâliyle görünür (model çıktısı gizlenmez).
  if (!parsed) {
    return (
      <pre className="g11-confidence-raw">
        <code>{code}</code>
      </pre>
    )
  }
  return (
    <p className={`g11-confidence g11-confidence-${parsed.level}`} role="note">
      <span className="g11-confidence-badge">Güven: {LABELS[parsed.level]}</span>
      {parsed.reason && <span className="g11-confidence-reason">{parsed.reason}</span>}
    </p>
  )
}

registerRichBlock('confidence', ConfidenceBlock)
