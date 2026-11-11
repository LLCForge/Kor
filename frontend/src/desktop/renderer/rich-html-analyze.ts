// [g8] Model üretimi HTML'in saf (DOM'suz, test edilebilir) statik çözümleyicisi. YALNIZCA bilgilendirme
//      amaçlıdır: engellenen dış istekleri, WebGL/form/canvas/özel eleman kullanımını, gömülü data: görselleri
//      ve temel erişilebilirlik uyarılarını sayar. Gerçek güvenlik sınırı sandbox + CSP'dir (SandboxedPreview);
//      bu çözümleyici onu DEĞİŞTİRMEZ, yalnızca kullanıcıya ne olduğunu/neyin engellendiğini açıklar.
//      Regex tabanlıdır (kasıtlı): test ortamında DOM yoktur ve bu, güvenlik kararı değil görüntüleme ipucudur.
import { MAX_RICH_SOURCE, withinRichSizeLimit } from './rich-sanitize.ts'

/** Bir data: URI başına izin verilen üst bayt sınırı (uyarı eşiği; gerçek sınır MAX_RICH_SOURCE kaynak boyutudur). */
export const MAX_DATA_URI_BYTES = 256 * 1024

/** Bir URL'nin kabaca sınıfı. 'external' ağ isteği doğuracak her şeyi kapsar (CSP tarafından engellenir). */
export type UrlClass = 'external' | 'data' | 'fragment' | 'relative' | 'blob' | 'about' | 'mailto' | 'empty'

export interface BlockedRef {
  url: string
  cls: UrlClass
}

export interface DataUri {
  mime: string
  bytes: number
}

export interface A11yIssue {
  /** Kısa, Türkçe uyarı metni. */
  message: string
}

export interface HtmlAnalysis {
  /** CSP'nin engelleyeceği kaynak referansları (dış, göreli, blob, about). */
  blockedRefs: BlockedRef[]
  /** <a> etiketlerindeki dış (http/https) bağlantı hedefleri (tıklama onayı/bilgi için). */
  externalLinks: string[]
  dataUris: DataUri[]
  dataUriTotalBytes: number
  /** Herhangi bir data: URI tekil sınırı aşıyorsa true. */
  dataUriOverLimit: boolean
  hasWebgl: boolean
  hasCanvas: boolean
  hasForm: boolean
  /** Betik açıkken ağ denemesi yapabilecek API kullanımı (yine de CSP ile engellenir). */
  usesNetworkApi: boolean
  /** Dış yazı tipi isteği belirtisi (CSP font-src data: ile engellenir). */
  usesExternalFont: boolean
  customElements: string[]
  a11yIssues: A11yIssue[]
  /** Kaba düğüm sayısı (açılış etiketi sayısı). */
  approxNodes: number
  /** Kaynak MAX_RICH_SOURCE'u aşıyorsa true (SandboxedPreview render etmez). */
  oversize: boolean
}

/** Bir URL'yi ağ/yerel/gömülü olarak sınıflar. Boşluk kırpılır. */
export function classifyUrl(raw: string): UrlClass {
  const u = raw.trim()
  if (!u) return 'empty'
  if (u.startsWith('#')) return 'fragment'
  if (/^data:/i.test(u)) return 'data'
  if (/^blob:/i.test(u)) return 'blob'
  if (/^about:/i.test(u)) return 'about'
  if (/^mailto:/i.test(u) || /^tel:/i.test(u)) return 'mailto'
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(u)) return 'external' // http://, https://, ftp://, ws(s)://
  if (u.startsWith('//')) return 'external' // protokol-göreli
  if (/^(?:javascript|vbscript):/i.test(u)) return 'external' // aktif şema (sanitize/CSP ele alır)
  return 'relative'
}

/** Opak origin + CSP default-src 'none' altında ağa/dış kaynağa gidip engellenecek sınıflar. */
function isBlockedClass(cls: UrlClass): boolean {
  return cls === 'external' || cls === 'relative' || cls === 'blob' || cls === 'about'
}

/** data: URI'nin MIME tipini ve yaklaşık bayt boyutunu çözer. */
export function measureDataUri(uri: string): DataUri {
  const comma = uri.indexOf(',')
  if (comma < 0) return { mime: '', bytes: 0 }
  const header = uri.slice('data:'.length, comma)
  const body = uri.slice(comma + 1)
  const isBase64 = /;base64/i.test(header)
  const mime = header.split(';')[0]?.trim() || 'text/plain'
  let bytes: number
  if (isBase64) {
    const clean = body.replace(/\s/g, '').replace(/=+$/, '')
    bytes = Math.floor((clean.length * 3) / 4)
  } else {
    try {
      bytes = decodeURIComponent(body).length
    } catch {
      bytes = body.length
    }
  }
  return { mime, bytes }
}

// Öznitelik referansları: src/href/srcset/poster/background/data/action/formaction/cite.
const ATTR_REF = /\b(?:src|href|srcset|poster|background|data|action|formaction|cite)\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))/gi
// CSS url(...) referansları (stil ve @font-face dahil).
const CSS_URL = /url\(\s*(['"]?)([^'")]+)\1\s*\)/gi
// data: URI'leri (base64 ve yüzde-kodlu; tırnak/boşluk/kapatma görünce biter).
const DATA_URI = /data:[a-z0-9.+-]*\/?[a-z0-9.+-]*(?:;[a-z0-9=.-]+)*,[^"')\s>]*/gi
// <a href="..."> hedefleri.
const ANCHOR_HREF = /<a\b[^>]*?\bhref\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))/gi
// <img ...> etiketleri (alt denetimi için).
const IMG_TAG = /<img\b[^>]*>/gi
// Tireli özel eleman etiketleri (ör. <my-widget>). Bilinen istisnalar hariç tutulur.
const CUSTOM_EL = /<([a-z][a-z0-9]*-[a-z0-9-]*)\b/gi
const NETWORK_API = /\b(?:fetch\s*\(|XMLHttpRequest|WebSocket|EventSource|sendBeacon|navigator\.connection|import\s*\()/
const WEBGL = /getContext\(\s*['"](?:webgl2?|experimental-webgl)['"]|\bWebGL2?RenderingContext\b/i

/** İlk tanımlı yakalama grubunu seçer (çift/tek tırnak/tırnaksız). */
function pickGroup(m: RegExpMatchArray): string {
  return m[2] ?? m[3] ?? m[4] ?? ''
}

/**
 * Model üretimi HTML'i statik olarak çözümler. Saftır: DOM kullanmaz, yan etkisi yoktur. Yalnızca görüntüleme
 * ipuçları üretir; hiçbir çıktısı güvenlik kararı değildir (asıl koruma sandbox + CSP'dir).
 */
export function analyzeHtml(source: string): HtmlAnalysis {
  const blocked: BlockedRef[] = []
  const seen = new Set<string>()
  const addRef = (raw: string): void => {
    const url = raw.trim()
    if (!url) return
    const cls = classifyUrl(url)
    if (!isBlockedClass(cls)) return
    const key = `${cls}:${url}`
    if (seen.has(key)) return
    seen.add(key)
    blocked.push({ url, cls })
  }

  for (const m of source.matchAll(ATTR_REF)) {
    const value = pickGroup(m)
    // srcset birden çok aday içerir: "a.png 1x, b.png 2x" → her adayın URL'sini al.
    if (/\bsrcset\s*=/i.test(m[0])) {
      for (const part of value.split(',')) addRef(part.trim().split(/\s+/)[0] ?? '')
    } else {
      addRef(value)
    }
  }
  for (const m of source.matchAll(CSS_URL)) addRef(m[2] ?? '')

  const externalLinks: string[] = []
  const linkSeen = new Set<string>()
  for (const m of source.matchAll(ANCHOR_HREF)) {
    const href = pickGroup(m).trim()
    if (classifyUrl(href) === 'external' && !linkSeen.has(href)) {
      linkSeen.add(href)
      externalLinks.push(href)
    }
  }

  const dataUris: DataUri[] = []
  let dataUriTotalBytes = 0
  let dataUriOverLimit = false
  for (const m of source.matchAll(DATA_URI)) {
    const d = measureDataUri(m[0])
    dataUris.push(d)
    dataUriTotalBytes += d.bytes
    if (d.bytes > MAX_DATA_URI_BYTES) dataUriOverLimit = true
  }

  const customSet = new Set<string>()
  for (const m of source.matchAll(CUSTOM_EL)) {
    const tag = (m[1] ?? '').toLowerCase()
    // Standart dışı ama özel eleman olmayan bazı etiketleri ele: yalnızca tireli adları say.
    if (tag.includes('-')) customSet.add(tag)
  }

  const a11yIssues: A11yIssue[] = []
  let imagesWithoutAlt = 0
  for (const m of source.matchAll(IMG_TAG)) {
    if (!/\balt\s*=/i.test(m[0])) imagesWithoutAlt++
  }
  if (imagesWithoutAlt > 0) a11yIssues.push({ message: `${imagesWithoutAlt} görselde alt metni yok` })
  const emptyLinks = (source.match(/<a\b[^>]*>\s*<\/a>/gi) ?? []).length
  if (emptyLinks > 0) a11yIssues.push({ message: `${emptyLinks} bağlantıda erişilebilir metin yok` })

  const usesExternalFont = /@font-face/i.test(source) && blocked.some((b) => /\.(?:woff2?|ttf|otf|eot)(?:[?#]|$)/i.test(b.url))

  return {
    blockedRefs: blocked,
    externalLinks,
    dataUris,
    dataUriTotalBytes,
    dataUriOverLimit,
    hasWebgl: WEBGL.test(source),
    hasCanvas: /<canvas\b/i.test(source),
    hasForm: /<form\b/i.test(source),
    usesNetworkApi: NETWORK_API.test(source),
    usesExternalFont,
    customElements: [...customSet],
    a11yIssues,
    approxNodes: (source.match(/<[a-z!/]/gi) ?? []).length,
    oversize: !withinRichSizeLimit(source),
  }
}

/** Bayt sayısını kısa, okunur bir dizeye çevirir (ör. "12,3 KB"). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1).replace('.', ',')} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1).replace('.', ',')} MB`
}

/** Kaynak boyut sınırı (yeniden dışa aktarım; UI ileti/eşik için). */
export { MAX_RICH_SOURCE }
