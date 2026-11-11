// [g10] Güvenilirlik yapı taşları: devre kesici, jitter'lı üstel geri çekilme, yeniden deneme bütçesi,
// hata sınıflama ve araç argümanı JSON onarımı. Saf modül: Electron'a ve oturuma bağımlı değildir.

/** Arayüze (olay köprüsü) ve CLI'ya bildirilen güvenilirlik olayları. */
export type G10Event =
  | { kind: 'circuitOpen'; target: string; retryInSec: number }
  | { kind: 'toolTimeout'; tool: string; seconds: number; reason: string }
  | { kind: 'reconnect'; attempt: number; continued: boolean }
  | { kind: 'degraded'; from: string; to: string; seconds: number }
  | { kind: 'contextTrimmed'; ok: boolean }
  | { kind: 'jsonRepaired'; tool: string; fixes: string[] }
  | { kind: 'refusal'; reason: string }
  | { kind: 'deadLetter'; title: string }
  | { kind: 'conflict'; path: string }
  | { kind: 'checkpoint'; files: number; partial: boolean }

// ---------- Devre kesici ----------

export type BreakerState = 'closed' | 'open' | 'halfOpen'

export interface BreakerOptions {
  /** Art arda bu kadar geçici hata devreyi açar */
  failureThreshold: number
  /** Açık kalma süresi; sonra tek bir deneme çağrısına izin verilir (yarı açık) */
  cooldownMs: number
  now?: () => number
}

export class CircuitOpenError extends Error {
  readonly retryInMs: number
  constructor(target: string, retryInMs: number) {
    super(
      `"${target}" art arda başarısız olduğu için geçici olarak devre dışı; yaklaşık ${Math.max(1, Math.ceil(retryInMs / 1000))} sn sonra yeniden denenecek. ` +
        'Bu sürede aynı işlemi tekrarlama; başka bir yol dene veya kullanıcıya bildir.',
    )
    this.name = 'CircuitOpenError'
    this.retryInMs = retryInMs
  }
}

export class CircuitBreaker {
  readonly name: string
  readonly #opts: BreakerOptions
  #failures = 0
  #openedAt: number | null = null
  #trial = false

  constructor(name: string, opts: BreakerOptions) {
    this.name = name
    this.#opts = opts
  }

  #now(): number {
    return this.#opts.now?.() ?? Date.now()
  }

  get state(): BreakerState {
    if (this.#openedAt === null) return 'closed'
    return this.#now() - this.#openedAt >= this.#opts.cooldownMs ? 'halfOpen' : 'open'
  }

  retryInMs(): number {
    return this.#openedAt === null ? 0 : Math.max(0, this.#opts.cooldownMs - (this.#now() - this.#openedAt))
  }

  /** Çağrıya izin verilirse true. Yarı açıkta aynı anda yalnızca bir deneme geçer. */
  tryPass(): boolean {
    const state = this.state
    if (state === 'closed') return true
    if (state === 'open' || this.#trial) return false
    this.#trial = true
    return true
  }

  assertPass(): void {
    if (!this.tryPass()) throw new CircuitOpenError(this.name, this.retryInMs() || this.#opts.cooldownMs)
  }

  success(): void {
    this.#failures = 0
    this.#openedAt = null
    this.#trial = false
  }

  /** Sayılmayan sonuç (ör. modelin yanlış argümanı): yarı açıktaki deneme hakkı geri verilir, durum değişmez. */
  release(): void {
    this.#trial = false
  }

  /** true: bu hata devreyi açtı */
  failure(): boolean {
    this.#failures++
    if (!this.#trial && this.#failures < this.#opts.failureThreshold) return false
    const opening = this.#openedAt === null || this.#trial
    this.#openedAt = this.#now()
    this.#trial = false
    return opening
  }
}

// ---------- Geri çekilme ve bütçe ----------

/** "Tam jitter": [0, min(max, base·2^attempt)] aralığından rastgele bekleme; eşzamanlı istemciler aynı anda yığılmaz. */
export function backoffDelay(attempt: number, baseMs: number, maxMs: number, random: () => number = Math.random): number {
  const ceiling = Math.min(maxMs, baseMs * 2 ** Math.max(0, attempt))
  return Math.floor(random() * ceiling)
}

/** Tur başına toplam yeniden deneme sayısı ve bekleme süresi sınırı (sonsuz tekrar döngüsüne karşı). */
export class RetryBudget {
  #retries: number
  #waitMs: number
  constructor(retries: number, waitMs: number) {
    this.#retries = retries
    this.#waitMs = waitMs
  }
  take(delayMs: number): boolean {
    if (this.#retries <= 0 || this.#waitMs < delayMs) return false
    this.#retries--
    this.#waitMs -= delayMs
    return true
  }
  get remaining(): number {
    return this.#retries
  }
}

export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason instanceof Error ? signal.reason : new Error('İptal edildi'))
      return
    }
    const onAbort = () => {
      clearTimeout(timer)
      reject(signal?.reason instanceof Error ? signal.reason : new Error('İptal edildi'))
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    signal?.addEventListener('abort', onAbort, { once: true })
  })
}

export interface RetryOptions {
  retries: number
  baseMs: number
  maxMs: number
  isRetryable: (err: unknown) => boolean
  signal?: AbortSignal
  budget?: RetryBudget
  random?: () => number
  onRetry?: (err: unknown, attempt: number, delayMs: number) => void
}

export async function retryWithBackoff<T>(fn: (attempt: number) => Promise<T>, o: RetryOptions): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await fn(attempt)
    } catch (err) {
      if (o.signal?.aborted || attempt >= o.retries || !o.isRetryable(err)) throw err
      const delay = backoffDelay(attempt, o.baseMs, o.maxMs, o.random)
      if (o.budget && !o.budget.take(delay)) throw err
      o.onRetry?.(err, attempt + 1, delay)
      await sleep(delay, o.signal)
    }
  }
}

// ---------- Hata sınıflama ----------

const TRANSIENT_CODES = new Set([
  'ECONNRESET',
  'ECONNREFUSED',
  'ECONNABORTED',
  'ETIMEDOUT',
  'EPIPE',
  'EAI_AGAIN',
  'ENETUNREACH',
  'ENETDOWN',
  'EHOSTUNREACH',
  'EBUSY',
  'EAGAIN',
  'EMFILE',
  'ENFILE',
  'UND_ERR_SOCKET',
  'UND_ERR_CLOSED',
  'UND_ERR_CONNECT_TIMEOUT',
  'UND_ERR_HEADERS_TIMEOUT',
  'UND_ERR_BODY_TIMEOUT',
])
const TRANSIENT_NAMES = new Set(['APIConnectionError', 'APIConnectionTimeoutError', 'TimeoutError', 'ToolTimeoutError'])

function field(err: unknown, key: string): unknown {
  return typeof err === 'object' && err !== null ? (err as Record<string, unknown>)[key] : undefined
}

/** Ağ kopması, zaman aşımı, kilitli dosya, 408/429/5xx: aynı işlemi kısa süre sonra tekrarlamak anlamlıdır. */
export function isTransientError(err: unknown, depth = 0): boolean {
  if (depth > 3 || typeof err !== 'object' || err === null) return false
  const code = field(err, 'code')
  if (typeof code === 'string' && TRANSIENT_CODES.has(code)) return true
  const name = field(err, 'name')
  if (typeof name === 'string' && TRANSIENT_NAMES.has(name)) return true
  const status = field(err, 'status')
  if (typeof status === 'number' && (status === 408 || status === 429 || status >= 500)) return true
  const message = field(err, 'message')
  // undici: soket akış ortasında kapanınca "terminated", bağlantı kurulamayınca "fetch failed".
  if (name === 'TypeError' && (message === 'terminated' || message === 'fetch failed')) return true
  return isTransientError(field(err, 'cause'), depth + 1)
}

/**
 * Venice bağlam taşması: HTTP 400 + code "context_length_exceeded" veya "TOO_MANY_TOKENS"
 * (docs.venice.ai, Error Codes: "Your request exceeds the model's maximum context").
 */
export function isContextOverflowError(err: unknown): boolean {
  if (field(err, 'status') !== 400) return false
  const inner = field(err, 'error')
  const codes = [field(err, 'code'), field(inner, 'code')]
  if (codes.some((c) => c === 'context_length_exceeded' || c === 'TOO_MANY_TOKENS')) return true
  const message = [field(err, 'message'), field(inner, 'message')].filter((m) => typeof m === 'string').join(' ')
  return /maximum context|context length|context window|too many tokens/i.test(message)
}

// ---------- JSON onarımı ----------

export type JsonRepair = { ok: true; text: string; fixes: string[] } | { ok: false; reason: string }

const CONTROL_ESCAPES: Record<string, string> = { '\n': '\\n', '\r': '\\r', '\t': '\\t', '\b': '\\b', '\f': '\\f' }

function isPlainObject(value: unknown): boolean {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * Modelin ürettiği bozuk araç argümanını yalnızca VERİYİ DEĞİŞTİRMEYEN düzeltmelerle onarır:
 * kod çiti, nesne dışındaki metin, sondaki virgül, dize içindeki çıplak satır sonu/sekme, Python sabitleri (True/False/None).
 * Kesik (kapanmamış) JSON asla tamamlanmaz: yarım içerikle dosya yazmak sessiz veri kaybıdır.
 */
export function repairJsonArguments(raw: string): JsonRepair {
  const fixes = new Set<string>()
  let text = raw.trim()
  const fence = /^```[a-zA-Z]*\s*([\s\S]*?)\s*```$/.exec(text)
  if (fence) {
    text = fence[1] ?? ''
    fixes.add('kod çiti kaldırıldı')
  }
  const start = text.indexOf('{')
  if (start < 0) return { ok: false, reason: 'Argümanlarda JSON nesnesi yok.' }
  if (start > 0) fixes.add('nesne öncesi metin atıldı')

  let out = ''
  let depth = 0
  let inString = false
  let escaped = false
  let end = -1
  for (let i = start; i < text.length; i++) {
    const ch = text.charAt(i)
    if (inString) {
      if (escaped) {
        escaped = false
        out += ch
      } else if (ch === '\\') {
        escaped = true
        out += ch
      } else if (ch === '"') {
        inString = false
        out += ch
      } else if (ch < ' ') {
        out += CONTROL_ESCAPES[ch] ?? `\\u${ch.charCodeAt(0).toString(16).padStart(4, '0')}`
        fixes.add('dize içindeki kontrol karakterleri kaçışlandı')
      } else {
        out += ch
      }
      continue
    }
    if (ch === '"') {
      inString = true
      out += ch
    } else if (ch === '{' || ch === '[') {
      depth++
      out += ch
    } else if (ch === '}' || ch === ']') {
      depth--
      out += ch
      if (depth === 0) {
        end = i
        break
      }
    } else if (ch === ',') {
      const next = /^\s*([}\]])/.exec(text.slice(i + 1))
      if (next) fixes.add('sondaki virgül kaldırıldı')
      else out += ch
    } else {
      const word = /^(True|False|None)\b/.exec(text.slice(i))
      const prev = out.at(-1) ?? ''
      if (word && !/[A-Za-z0-9_]/.test(prev)) {
        const literal = word[1] === 'True' ? 'true' : word[1] === 'False' ? 'false' : 'null'
        out += literal
        i += (word[1] ?? '').length - 1
        fixes.add('Python sabitleri JSON karşılığına çevrildi')
      } else {
        out += ch
      }
    }
  }
  if (end < 0) return { ok: false, reason: 'JSON kapanmamış; yanıt kesilmiş olabilir. İçeriği daha küçük parçalara bölerek yeniden gönder.' }
  const rest = text.slice(end + 1)
  if (/[^\s`}\]]/.test(rest)) return { ok: false, reason: 'JSON nesnesinden sonra fazladan içerik var (birden fazla nesne birleşmiş olabilir).' }
  if (rest.trim()) fixes.add('fazladan kapanış karakterleri atıldı')

  try {
    const value: unknown = JSON.parse(out)
    if (!isPlainObject(value)) return { ok: false, reason: 'Argümanlar bir JSON nesnesi olmalı.' }
    return { ok: true, text: out, fixes: [...fixes] }
  } catch (err) {
    return { ok: false, reason: `JSON ayrıştırılamadı: ${err instanceof Error ? err.message : String(err)}` }
  }
}

/** Boş argüman ajan döngüsünde {} sayılır. */
export function isValidJsonObject(text: string): boolean {
  if (text.trim() === '') return true
  try {
    return isPlainObject(JSON.parse(text))
  } catch {
    return false
  }
}
