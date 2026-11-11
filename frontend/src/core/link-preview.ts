// [g3] Bağlantı önizlemesi: yalnızca http(s), yalnızca genel (public) IP'ler, zaman aşımı ve boyut sınırıyla.
import { lookup } from 'node:dns'
import http from 'node:http'
import https from 'node:https'
import { BlockList, isIP, type LookupFunction } from 'node:net'

export interface LinkPreview {
  url: string
  title: string | null
  description: string | null
  siteName: string | null
}

const TIMEOUT_MS = 6_000
const MAX_BYTES = 256 * 1024
const MAX_REDIRECTS = 3
const MAX_URL_CHARS = 2048

// İç ağa, bulut meta veri servisine (169.254.169.254) ve özel adreslere istek atılmaz (CWE-918 SSRF).
const blocked = new BlockList()
for (const [net, prefix] of [
  ['0.0.0.0', 8],
  ['10.0.0.0', 8],
  ['100.64.0.0', 10],
  ['127.0.0.0', 8],
  ['169.254.0.0', 16],
  ['172.16.0.0', 12],
  ['192.0.0.0', 24],
  ['192.0.2.0', 24],
  ['192.88.99.0', 24],
  ['192.168.0.0', 16],
  ['198.18.0.0', 15],
  ['198.51.100.0', 24],
  ['203.0.113.0', 24],
  ['224.0.0.0', 4],
  ['240.0.0.0', 4],
] as const) {
  blocked.addSubnet(net, prefix, 'ipv4')
}
for (const [net, prefix] of [
  ['::', 128],
  ['::1', 128],
  ['64:ff9b:1::', 48],
  ['100::', 64],
  ['2001:db8::', 32],
  ['fc00::', 7],
  ['fe80::', 10],
  ['fec0::', 10],
  ['ff00::', 8],
] as const) {
  blocked.addSubnet(net, prefix, 'ipv6')
}

/** IPv6 içine gömülü IPv4 (::ffff:a.b.c.d, NAT64 64:ff9b::/96, 6to4 2002::/16) ayrıca denetlenir. */
function embeddedIpv4(ip: string): string | null {
  const lower = ip.toLowerCase()
  const dotted = /^(?:::ffff:|64:ff9b::|::)(\d+\.\d+\.\d+\.\d+)$/.exec(lower)
  if (dotted?.[1]) return dotted[1]
  const hex = /^(?:::ffff:|64:ff9b::)([0-9a-f]{1,4}):([0-9a-f]{1,4})$/.exec(lower)
  if (hex?.[1] && hex[2]) {
    const hi = Number.parseInt(hex[1], 16)
    const lo = Number.parseInt(hex[2], 16)
    return `${hi >> 8}.${hi & 255}.${lo >> 8}.${lo & 255}`
  }
  const sixToFour = /^2002:([0-9a-f]{1,4}):([0-9a-f]{1,4}):/.exec(lower)
  if (sixToFour?.[1] && sixToFour[2]) {
    const hi = Number.parseInt(sixToFour[1], 16)
    const lo = Number.parseInt(sixToFour[2], 16)
    return `${hi >> 8}.${hi & 255}.${lo >> 8}.${lo & 255}`
  }
  return null
}

export function isPublicAddress(ip: string): boolean {
  const family = isIP(ip)
  if (family === 4) return !blocked.check(ip, 'ipv4')
  if (family !== 6) return false
  const v4 = embeddedIpv4(ip)
  if (v4 !== null) return isPublicAddress(v4)
  return !blocked.check(ip, 'ipv6')
}

/** Kimlik bilgisi içeren, http(s) olmayan veya standart dışı porta giden adresler reddedilir. */
export function validatePreviewUrl(raw: string): URL {
  if (raw.length > MAX_URL_CHARS) throw new Error('Adres çok uzun')
  let url: URL
  try {
    url = new URL(raw)
  } catch {
    throw new Error('Geçersiz adres')
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error('Yalnızca http/https bağlantıları önizlenebilir')
  if (url.username || url.password) throw new Error('Kimlik bilgisi içeren adresler önizlenmez')
  if (url.port !== '') throw new Error('Yalnızca standart portlar önizlenebilir')
  const host = url.hostname.replace(/^\[|\]$/g, '')
  if (host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local') || host.endsWith('.internal')) {
    throw new Error('Yerel adresler önizlenmez')
  }
  if (isIP(host) && !isPublicAddress(host)) throw new Error('Özel/yerel IP adresleri önizlenmez')
  url.hash = ''
  return url
}

/**
 * DNS çözümlemesi bağlantı anında denetlenir: önce çözüp sonra fetch etmek DNS rebinding'e açıktır,
 * burada soket yalnızca denetlenmiş adrese bağlanır.
 */
const safeLookup: LookupFunction = (hostname, options, callback) => {
  lookup(hostname, { ...options, all: true }, (err, addresses) => {
    if (err) {
      callback(err, '', 4)
      return
    }
    const list = Array.isArray(addresses) ? addresses : []
    const bad = list.find((a) => !isPublicAddress(a.address))
    const first = list[0]
    if (bad || !first) {
      callback(Object.assign(new Error('Özel/yerel IP adreslerine istek atılmaz'), { code: 'EG3PRIVATE' }), '', 4)
      return
    }
    if (options.all) callback(null, list)
    else callback(null, first.address, first.family)
  })
}

interface RawResponse {
  status: number
  location: string | null
  contentType: string
  body: string
}

function request(url: URL, signal: AbortSignal): Promise<RawResponse> {
  const options: http.RequestOptions = {
    lookup: safeLookup,
    signal,
    headers: { 'User-Agent': 'Kor-LinkPreview/1.0', Accept: 'text/html,application/xhtml+xml', 'Accept-Encoding': 'identity' },
  }
  return new Promise((resolve, reject) => {
    const onResponse = (res: http.IncomingMessage) => {
      const status = res.statusCode ?? 0
      const contentType = String(res.headers['content-type'] ?? '')
      const location = typeof res.headers.location === 'string' ? res.headers.location : null
      if (status >= 300 && status < 400) {
        res.resume()
        resolve({ status, location, contentType, body: '' })
        return
      }
      if (!/text\/html|application\/xhtml\+xml/i.test(contentType)) {
        res.destroy()
        resolve({ status, location: null, contentType, body: '' })
        return
      }
      const chunks: Buffer[] = []
      let size = 0
      res.on('data', (chunk: Buffer) => {
        size += chunk.length
        chunks.push(chunk)
        // Başlık bilgileri belgenin başındadır; sınırı aşınca bağlantı kesilir.
        if (size >= MAX_BYTES) res.destroy()
      })
      const finish = () => resolve({ status, location: null, contentType, body: Buffer.concat(chunks).subarray(0, MAX_BYTES).toString('utf8') })
      res.on('end', finish)
      res.on('close', finish)
      res.on('error', reject)
    }
    const req = url.protocol === 'https:' ? https.get(url, options, onResponse) : http.get(url, options, onResponse)
    req.on('error', reject)
  })
}

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' }

function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, code: string) => {
    if (code[0] === '#') {
      const n = code[1]?.toLowerCase() === 'x' ? Number.parseInt(code.slice(2), 16) : Number.parseInt(code.slice(1), 10)
      return Number.isFinite(n) && n > 0 && n <= 0x10ffff ? String.fromCodePoint(n) : whole
    }
    return ENTITIES[code.toLowerCase()] ?? whole
  })
}

function clean(value: string | undefined, max: number): string | null {
  if (!value) return null
  const text = decodeEntities(value).replace(/\s+/g, ' ').trim()
  if (!text) return null
  return text.length > max ? `${text.slice(0, max - 1)}…` : text
}

/** Yalnızca og:title, og:description, og:site_name (ve yedek olarak <title>, meta description) okunur; görsel yok. */
export function parseLinkPreview(html: string, url: string): LinkPreview {
  const head = html.slice(0, MAX_BYTES)
  const meta = new Map<string, string>()
  for (const tag of head.matchAll(/<meta\b[^>]*>/gi)) {
    const attrs = new Map<string, string>()
    for (const m of tag[0].matchAll(/([a-z:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))/gi)) {
      attrs.set((m[1] ?? '').toLowerCase(), m[2] ?? m[3] ?? m[4] ?? '')
    }
    const key = (attrs.get('property') ?? attrs.get('name'))?.toLowerCase()
    const content = attrs.get('content')
    if (key && content !== undefined && !meta.has(key)) meta.set(key, content)
  }
  const titleTag = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(head)?.[1]
  return {
    url,
    title: clean(meta.get('og:title') ?? titleTag, 200),
    description: clean(meta.get('og:description') ?? meta.get('description'), 400),
    siteName: clean(meta.get('og:site_name'), 80),
  }
}

/** Yönlendirmeler elle izlenir; her adımda adres yeniden doğrulanır. */
export async function fetchLinkPreview(raw: string): Promise<LinkPreview | null> {
  const signal = AbortSignal.timeout(TIMEOUT_MS)
  let url = validatePreviewUrl(raw)
  for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
    const res = await request(url, signal)
    if (res.status >= 300 && res.status < 400) {
      if (!res.location) return null
      url = validatePreviewUrl(new URL(res.location, url).toString())
      continue
    }
    if (res.status < 200 || res.status >= 300 || !res.body) return null
    return parseLinkPreview(res.body, url.toString())
  }
  throw new Error('Çok fazla yönlendirme')
}
