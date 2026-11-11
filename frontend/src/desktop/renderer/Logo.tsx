import { BRAND_SVG, stillSvg, svgUrl } from './brand/brand.ts'
import { useMotionStill } from './motion.ts'

const TILE = svgUrl(BRAND_SVG.appIconRounded)
const MARK = svgUrl(BRAND_SVG.markColor)
const MARK_CRISP = svgUrl(BRAND_SVG.markColorCrisp)
const PULSE = svgUrl(BRAND_SVG.emberPulse)
const PULSE_STILL = svgUrl(stillSvg(BRAND_SVG.emberPulse))
/** Bu boyut ve altında parıltı katmanı bulanıklık yaratır; keskin işaret çizilir. */
const CRISP_MAX = 32

/**
 * Kor logosu, marka kitinden (assets/). Varsayılan: arka plansız kor işareti (açık ve koyu zeminde net).
 * tile=true: koyu karo üzerinde işaret (uygulama ikonu; yalnızca ikon gerektiren yerlerde).
 * <img> ile çizilir: SVG gradyan kimlikleri birden çok logo arasında çakışmaz.
 */
export function Logo({ size = 28, tile = false, className }: { size?: number; tile?: boolean; className?: string }) {
  return (
    <img
      src={tile ? TILE : size <= CRISP_MAX ? MARK_CRISP : MARK}
      width={size}
      height={size}
      className={className}
      alt="Kor"
      draggable={false}
      style={{ objectFit: 'contain', display: 'block' }}
    />
  )
}

/** Çalışırken nabız gibi atan kor (marka kitindeki ember-pulse animasyonu); hareket kapalıyken durağan kare. */
export function EmberPulse({ size = 18, label = 'Çalışıyor' }: { size?: number; label?: string }) {
  const still = useMotionStill()
  return <img src={still ? PULSE_STILL : PULSE} width={size} height={size} alt={label} draggable={false} />
}
