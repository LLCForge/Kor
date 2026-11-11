/**
 * Kor marka kiti: logo SVG'leri ve kıvılcım spinner kareleri doğrudan /assets'ten okunur (kopya yok; spec §16.2
 * kural 1, ADR 0053). Renkler assets/theme.json'dan; düşünme fiili çekirdeğin `agent.status` olayından gelir
 * (agent-status.ts), burada fiil seçimi yoktur.
 */
import theme from '../../../../../assets/theme.json' with { type: 'json' }
import markColor from '../../../../../assets/kor-mark-color.svg'
import markMono from '../../../../../assets/kor-mark-mono.svg'
import appIconRounded from '../../../../../assets/app-icon-rounded.svg'
import logoAuto from '../../../../../assets/kor-logo-auto.svg'
import emberPulse from '../../../../../assets/ember-pulse.svg'
import spinnerEmber from '../../../../../assets/spinner-ember.svg'
import thinkingCube from '../../../../../assets/thinking-cube.svg'

/** Kıvılcım kareleri: ✶ ✷ ✸ ✹ ✺ ✹ ✸ ✷ */
export const EMBER_FRAMES: readonly string[] = theme.frames.ember
/** Kare başına renk; spinner kor gibi yanıp söner. */
export const EMBER_GRADIENT: readonly string[] = theme.gradient

/** SVG metnini <img src> için veri URL'sine çevirir (satır içi HTML enjeksiyonu yok; CSP img-src data:). */
export function svgUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

/**
 * İşaretin bulanık parıltı katmanı (feGaussianBlur, filter="url(#kglow)") küçük boyutta (başlık çubuğu 18 px) işareti
 * bulanık gösterir; o boyutlarda parıltısız, keskin kopya kullanılır. Asset dosyası değişmez.
 */
export function withoutGlow(svg: string): string {
  return svg.replace(/<path\b[^>]*filter="url\(#kglow\)"[^>]*\/>/g, '')
}

export { stillSvg } from './still.ts'
export { KorThinking, KorSpinner, KorDots } from 'kor-assets/KorThinking'

export const BRAND_SVG = {
  markColor,
  markColorCrisp: withoutGlow(markColor),
  markMono,
  appIconRounded,
  logoAuto,
  emberPulse,
  spinnerEmber,
  thinkingCube,
} as const
