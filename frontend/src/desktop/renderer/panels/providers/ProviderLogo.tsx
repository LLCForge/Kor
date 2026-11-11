// §23.2: her model adının solunda sağlayıcının logosu (assets/providers/<id>.svg). Tek renk (currentColor) ve tam
// renkli iki varyant; bilinmeyen sağlayıcı ember çemberde harf monogramı; yerel modeller yerel ikonu + "yerel"
// çipi; OpenRouter üzerinden gelen modellerde asıl sağlayıcının logosu + küçük OpenRouter rozeti.
import { HardDrive } from 'lucide-react'
import { BrandSvg, brandSvgForText } from '../../g9/BrandIcons.tsx'
import { PROVIDER_LOGOS } from '../../provider-logos.gen.ts'
import { logoPlan } from '../../provider-logo.ts'

export interface ProviderLogoProps {
  provider: string
  modelId?: string
  /** Tek renk (varsayılan; metin rengini alır) ya da markanın kendi renkleri */
  variant?: 'mono' | 'color'
  size?: number
  /** "yerel" / OpenRouter rozetlerini göster (dar yerlerde kapatılır) */
  badges?: boolean
}

function Mark({ svg, size }: { svg: string; size: number }) {
  return <BrandSvg svg={svg} size={size} />
}

export function ProviderLogo({ provider, modelId = '', variant = 'mono', size = 16, badges = true }: ProviderLogoProps) {
  const plan = logoPlan(provider, modelId, (p, m) => brandSvgForText(p, m) !== null)
  const svgFor = (p: string, m: string): string | null => {
    const mono = brandSvgForText(p, m)
    if (!mono || variant === 'mono') return mono
    const entry = Object.values(PROVIDER_LOGOS).find((l) => l.mono === mono)
    return entry?.color ?? mono
  }
  const main = plan.kind === 'local' ? null : svgFor(plan.provider, plan.modelId)
  return (
    <span className="kor-provider-logo-wrap" title={plan.title}>
      {plan.kind === 'local' ? (
        <HardDrive size={size} aria-hidden="true" className="kor-provider-local" />
      ) : main ? (
        <Mark svg={main} size={size} />
      ) : (
        <span className="kp-monogram kor-provider-monogram" aria-hidden="true" style={{ width: size, height: size, fontSize: Math.round(size * 0.62) }}>
          {plan.monogram}
        </span>
      )}
      {badges && plan.kind === 'local' && <span className="kor-provider-chip">yerel</span>}
      {badges && plan.via === 'openrouter' && PROVIDER_LOGOS.openrouter && (
        <span className="kor-provider-via" aria-label="OpenRouter üzerinden" title="OpenRouter üzerinden">
          <Mark svg={PROVIDER_LOGOS.openrouter.mono} size={Math.max(8, Math.round(size * 0.6))} />
        </span>
      )}
      <span className="sr-only">{plan.title}</span>
    </span>
  )
}
