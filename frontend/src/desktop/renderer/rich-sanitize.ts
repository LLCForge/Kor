// [temel] Zengin içerik güvenlik sınırının politika yardımcıları (saf, DOM'suz test edilebilir) ve SVG temizleme.
//
// Güvenlik modeli (fail-closed, allowlist, ağ yok):
//   - iframe belgesi ve CSP başlığı: ../sandbox-document.ts (ana süreç sunar); bayraklar SandboxedPreview.tsx.
//   - boyut sınırları: devasa kaynak (DoS) iframe'e/DOM'a girmeden reddedilir.
import DOMPurify from 'dompurify'

/** srcdoc/SVG kaynağı için bayt üst sınırı (DoS: aşırı büyük içerik render edilmez). */
export const MAX_RICH_SOURCE = 512 * 1024
/** Temizlenmiş SVG'de izin verilen en fazla düğüm sayısı (DoS: düğüm patlaması engellenir). */
export const MAX_SVG_NODES = 20_000

/** Kaynak boyut sınırını aşmıyorsa true. */
export function withinRichSizeLimit(source: string): boolean {
  return source.length <= MAX_RICH_SOURCE
}

/** Yalnızca güvenli renk değerleri geçer (tema renklerinin önizleme gövdesine CSS enjeksiyonunu önler). */
export function cssColor(value: string | undefined): string | undefined {
  if (!value) return undefined
  const v = value.trim()
  return /^#[0-9a-fA-F]{3,8}$|^(?:rgb|hsl)a?\([0-9.,%\s/]+\)$|^[a-zA-Z]{3,20}$/.test(v) ? v : undefined
}

/**
 * SVG referans özniteliği güvenlik kuralı (DOMPurify uponSanitizeAttribute kancasının saf çekirdeği).
 * Yalnızca belge içi (#id) referanslar korunur; http(s)://, //, data:, javascript: gibi dış/aktif
 * referanslar düşürülür. Referans dışı öznitelikler bu kuraldan etkilenmez.
 */
export function isSafeSvgRef(attrName: string, value: string): boolean {
  const name = attrName.toLowerCase()
  if (name !== 'href' && name !== 'xlink:href' && name !== 'src') return true
  return value.trim().startsWith('#')
}

export type SvgResult = { ok: true; html: string; nodes: number } | { ok: false; reason: string }

/** SVG'yi güvenli hale getirir: script/foreignObject kaldırılır, olay işleyicileri ve dış referanslar düşürülür. */
export function sanitizeSvg(svg: string): SvgResult {
  if (!withinRichSizeLimit(svg)) return { ok: false, reason: 'SVG görüntülenemeyecek kadar büyük.' }
  const purifier = DOMPurify(window)
  purifier.addHook('uponSanitizeAttribute', (_node, data) => {
    if (!isSafeSvgRef(data.attrName, data.attrValue)) data.keepAttr = false
  })
  const fragment = purifier.sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true },
    FORBID_TAGS: ['script', 'foreignObject'],
    RETURN_DOM_FRAGMENT: true,
  })
  const nodes = fragment.querySelectorAll('*').length
  if (nodes > MAX_SVG_NODES) return { ok: false, reason: 'SVG görüntülenemeyecek kadar karmaşık.' }
  const root = fragment.querySelector('svg')
  if (!root) return { ok: false, reason: 'Geçerli bir SVG bulunamadı.' }
  return { ok: true, html: root.outerHTML, nodes }
}
