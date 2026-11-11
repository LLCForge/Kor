import dns, { type LookupAddress } from 'node:dns'
import http, { type IncomingMessage } from 'node:http'
import https from 'node:https'
import { BlockList, isIP, type LookupFunction } from 'node:net'
import { pipeline, type Readable } from 'node:stream'
import zlib from 'node:zlib'
import type OpenAI from 'openai'
import { z } from 'zod'

export const MAX_FETCH_BYTES = 2 * 1024 * 1024
const FETCH_TIMEOUT_MS = 30_000
const MAX_REDIRECTS = 5

const blockedV4 = new BlockList()
for (const [net, prefix] of [
  ['0.0.0.0', 8], ['10.0.0.0', 8], ['100.64.0.0', 10], ['127.0.0.0', 8], ['169.254.0.0', 16], ['172.16.0.0', 12],
  ['192.0.0.0', 24], ['192.0.2.0', 24], ['192.88.99.0', 24], ['192.168.0.0', 16], ['198.18.0.0', 15],
  ['198.51.100.0', 24], ['203.0.113.0', 24], ['224.0.0.0', 4], ['240.0.0.0', 4],
] as const) {
  blockedV4.addSubnet(net, prefix, 'ipv4')
}

const blockedV6 = new BlockList()
for (const [net, prefix] of [
  ['::', 96], ['100::', 64], ['2001::', 32], ['2001:db8::', 32], ['2002::', 16],
  ['fc00::', 7], ['fe80::', 10], ['fec0::', 10], ['ff00::', 8],
] as const) {
  blockedV6.addSubnet(net, prefix, 'ipv6')
}

function ipv6Groups(ip: string): number[] | null {
  let s = ip
  const dotted = /(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(s)
  if (dotted) {
    const [a, b, c, d] = dotted.slice(1).map(Number) as [number, number, number, number]
    s = `${s.slice(0, dotted.index)}${((a << 8) | b).toString(16)}:${((c << 8) | d).toString(16)}`
  }
  const halves = s.split('::')
  if (halves.length > 2) return null
  const head = halves[0] ? halves[0].split(':') : []
  const tail = halves.length === 2 && halves[1] ? halves[1].split(':') : []
  const groups = halves.length === 2 ? [...head, ...Array<string>(8 - head.length - tail.length).fill('0'), ...tail] : head
  if (groups.length !== 8) return null
  return groups.map((g) => Number.parseInt(g, 16))
}

/**
 * SSRF koruması (CWE-918): özel, yerel, link-local, loopback, CGNAT, çoklu yayın ve bulut metadata
 * (169.254.169.254, fd00:ec2::254) adresleri reddedilir. IPv4'ü gömen IPv6 biçimleri gömülü adresle denetlenir.
 */
export function isBlockedAddress(address: string): boolean {
  const ip = address.split('%')[0] ?? ''
  const family = isIP(ip)
  if (family === 4) return blockedV4.check(ip, 'ipv4')
  if (family !== 6) return true
  const g = ipv6Groups(ip)
  if (!g) return true
  const mapped = g.slice(0, 5).every((x) => x === 0) && g[5] === 0xffff
  const nat64 = g[0] === 0x64 && g[1] === 0xff9b && g.slice(2, 6).every((x) => x === 0)
  if (mapped || nat64) {
    const hi = g[6] ?? 0
    const lo = g[7] ?? 0
    return isBlockedAddress(`${hi >> 8}.${hi & 255}.${lo >> 8}.${lo & 255}`)
  }
  return blockedV6.check(ip, 'ipv6')
}

/** Bağlantı anında çözümlenen adresleri denetler; denetlenen adres kullanılır (DNS rebinding'e karşı). */
function safeLookup(isBlocked: (ip: string) => boolean): LookupFunction {
  return (hostname, options, callback) => dns.lookup(hostname, { ...options, all: true }, (err, addresses: LookupAddress[]) => {
    if (err) {
      callback(err, '', 0)
      return
    }
    const bad = addresses.find((a) => isBlocked(a.address))
    const first = addresses[0]
    if (bad || !first) {
      const error: NodeJS.ErrnoException = new Error(`Özel/yerel ağ adresine erişim engellendi: ${hostname} (${bad?.address ?? 'adres yok'})`)
      error.code = 'ESSRF'
      callback(error, '', 0)
      return
    }
    if (options.all) callback(null, addresses)
    else callback(null, first.address, first.family)
  })
}

/** Şema, kimlik bilgisi ve IP sabiti kontrolü; alan adları bağlantı sırasında safeLookup ile denetlenir. */
export function assertFetchableUrl(raw: string | URL, isBlocked: (ip: string) => boolean = isBlockedAddress): URL {
  let url: URL
  try {
    url = new URL(raw)
  } catch {
    throw new Error(`Geçersiz URL: ${String(raw).slice(0, 200)}`)
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error('Yalnızca http ve https adresleri okunabilir')
  if (url.username || url.password) throw new Error('Kimlik bilgisi içeren URL reddedildi')
  const host = url.hostname.replace(/^\[|\]$/g, '')
  if (isIP(host) && isBlocked(host)) throw new Error(`Özel/yerel ağ adresine erişim engellendi: ${host}`)
  if (/^localhost$|\.localhost$/i.test(host)) throw new Error(`Yerel adrese erişim engellendi: ${host}`)
  return url
}

export interface FetchResult {
  finalUrl: string
  status: number
  contentType: string
  body: Buffer
  truncated: boolean
}

function request(url: URL, signal: AbortSignal, lookup: LookupFunction): Promise<IncomingMessage> {
  const lib = url.protocol === 'https:' ? https : http
  return new Promise((resolve, reject) => {
    const req = lib.request(
      url,
      {
        method: 'GET',
        signal,
        lookup,
        headers: {
          'User-Agent': 'Kor/1.0 (web_fetch)',
          Accept: 'text/html,application/xhtml+xml,text/plain,text/markdown,application/json;q=0.9,*/*;q=0.5',
          'Accept-Encoding': 'gzip, deflate, br',
        },
      },
      resolve,
    )
    req.on('error', reject)
    req.end()
  })
}

async function readLimited(res: IncomingMessage, maxBytes: number): Promise<{ body: Buffer; truncated: boolean }> {
  const encoding = String(res.headers['content-encoding'] ?? '').toLowerCase()
  let stream: Readable = res
  // Sınır açılmış veriye uygulanır: sıkıştırma bombasına karşı.
  const decompressor = encoding === 'gzip' || encoding === 'x-gzip' || encoding === 'deflate' ? zlib.createUnzip() : encoding === 'br' ? zlib.createBrotliDecompress() : null
  if (decompressor) {
    // pipe() kaynağın hatasını taşımaz: kesilen gövdede açıcı sonsuza kadar beklerdi. pipeline hatayı açıcıya geçirir, aşağıdaki döngü reddedilir.
    pipeline(res, decompressor, () => undefined)
    stream = decompressor
  }
  const chunks: Buffer[] = []
  let size = 0
  let truncated = false
  try {
    for await (const chunk of stream) {
      const buf = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk))
      if (size + buf.length > maxBytes) {
        chunks.push(buf.subarray(0, maxBytes - size))
        truncated = true
        break
      }
      chunks.push(buf)
      size += buf.length
    }
  } catch (err) {
    if (!decompressor) throw err
    throw new Error(`Sıkıştırılmış yanıt eksik veya bozuk: ${err instanceof Error ? err.message : String(err)}`)
  } finally {
    res.destroy()
    if (stream !== res) stream.destroy()
  }
  return { body: Buffer.concat(chunks), truncated }
}

export interface FetchOptions {
  maxBytes?: number
  /** Yalnızca testler için: yerel test sunucusuna izin vermek amacıyla adres denetimi değiştirilebilir. */
  isBlocked?: (ip: string) => boolean
}

/** Yönlendirmeleri elle izler: her adımda URL yeniden denetlenir (en fazla 5). */
export async function fetchUrl(raw: string, signal: AbortSignal, options: FetchOptions = {}): Promise<FetchResult> {
  const isBlocked = options.isBlocked ?? isBlockedAddress
  const lookup = safeLookup(isBlocked)
  const combined = AbortSignal.any([signal, AbortSignal.timeout(FETCH_TIMEOUT_MS)])
  let url = assertFetchableUrl(raw, isBlocked)
  try {
    for (let hop = 0; ; hop++) {
      const res = await request(url, combined, lookup)
      const status = res.statusCode ?? 0
      const location = res.headers.location
      if (status >= 300 && status < 400 && location) {
        res.destroy()
        if (hop >= MAX_REDIRECTS) throw new Error(`Çok fazla yönlendirme (sınır ${MAX_REDIRECTS})`)
        url = assertFetchableUrl(new URL(location, url), isBlocked)
        continue
      }
      const { body, truncated } = await readLimited(res, options.maxBytes ?? MAX_FETCH_BYTES)
      return { finalUrl: url.href, status, contentType: String(res.headers['content-type'] ?? ''), body, truncated }
    }
  } catch (err) {
    if (combined.aborted && !signal.aborted) throw new Error(`Zaman aşımı: ${FETCH_TIMEOUT_MS / 1000} sn içinde yanıt alınamadı`)
    throw err
  }
}

/** Gövdeyi content-type'taki karakter kümesiyle çözer; bilinmeyen küme UTF-8'e düşer. */
export function decodeBody(body: Buffer, contentType: string): string {
  const charset = /charset\s*=\s*["']?([\w-]+)/i.exec(contentType)?.[1] ?? 'utf-8'
  try {
    return new TextDecoder(charset).decode(body)
  } catch {
    return new TextDecoder('utf-8').decode(body)
  }
}

export function isTextual(contentType: string, body: Buffer): boolean {
  const type = contentType.split(';')[0]?.trim().toLowerCase() ?? ''
  if (!type) return !body.subarray(0, 4096).includes(0)
  return type.startsWith('text/') || /(json|xml|javascript|ecmascript|yaml|markdown|x-sh|csv)/.test(type)
}

/**
 * Uzun sayfada, prompt'taki terimlerle en çok örtüşen paragraflar (özgün sırasıyla) seçilir.
 * Model çağrısı gerektirmez: ek maliyet yok, sonuç belirlenimci.
 */
export function selectRelevant(markdown: string, prompt: string, limit: number): string {
  if (markdown.length <= limit) return markdown
  const terms = [...new Set(prompt.toLocaleLowerCase('tr').match(/[\p{L}\p{N}_-]{3,}/gu) ?? [])]
  if (terms.length === 0) return markdown
  const chunks = markdown.split(/\n{2,}/)
  const scored = chunks.map((text, index) => {
    const lower = text.toLocaleLowerCase('tr')
    // Başlıklar bağlam taşır: küçük bir öncelik alır.
    const score = terms.reduce((sum, t) => sum + (lower.split(t).length - 1), 0) + (text.startsWith('#') ? 0.5 : 0)
    return { text, index, score }
  })
  const picked: typeof scored = []
  let used = 0
  for (const chunk of [...scored].sort((a, b) => b.score - a.score || a.index - b.index)) {
    if (chunk.score <= 0 || used + chunk.text.length + 2 > limit) continue
    picked.push(chunk)
    used += chunk.text.length + 2
  }
  if (picked.length === 0) return markdown.slice(0, limit)
  picked.sort((a, b) => a.index - b.index)
  return `[Sayfa uzun: "${prompt.slice(0, 100)}" ile ilgili ${picked.length}/${chunks.length} bölüm seçildi]\n\n${picked.map((c) => c.text).join('\n\n')}`
}

const searchResponseSchema = z.object({
  results: z
    .array(
      z.object({
        title: z.string().max(2000).catch(''),
        url: z.string().max(4096),
        content: z.string().max(100_000).catch(''),
        date: z.string().max(100).nullish(),
      }),
    )
    .max(50),
})

export type SearchResult = z.infer<typeof searchResponseSchema>['results'][number]

/**
 * Venice web araması: POST /augment/search (belgeler: docs.venice.ai/api-reference/endpoint/augment/search).
 * Token harcamaz, istek başına sabit ücretlidir; sohbet tamamlama + enable_web_search yolundan ucuzdur.
 */
export async function veniceWebSearch(client: OpenAI, query: string, limit: number, signal: AbortSignal): Promise<SearchResult[]> {
  const raw = await client.post<unknown>('/augment/search', { body: { query, limit }, signal, timeout: FETCH_TIMEOUT_MS })
  return searchResponseSchema.parse(raw).results
}
