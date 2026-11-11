// [g20] İzole JS çalıştırıcı (GÜVENLİK SINIRI). Model kodu ana belgede çalışmaz; her çalıştırma yeni bir blob: Worker'da.
//
// Savunma katmanları (ağ engeli scripts/g20-verify-worker-net.ts ile gerçek Electron'da kanıtlanır):
//   1. blob: worker, ana belgenin CSP'sini DEVRALIR: connect-src 'none' (fetch/XHR/WebSocket/EventSource yok),
//      script-src 'self' (importScripts/import() ile dış betik yok, eval yok). file:// worker devralmaz; bu yüzden blob:.
//   2. Önsöz, ağ/depolama/iç içe worker global'lerini siler (g20-repl-core.ts workerPrelude).
//   3. Worker'da DOM yoktur; ana tarafa yalnızca doğrulanmış mesajlar metin olarak işlenir.
//   4. Zaman aşımında terminate(); çıktı hem worker'da hem burada sınırlanır.
//   Bellek: tarayıcı worker'ına yığın sınırı verilemez (resourceLimits yalnız Node'da); kaçak ayırma zaman aşımıyla kesilir.
import { buildWorkerScript, SCRIPT_MODES, type VarInfo } from './g20-repl-core.ts'

export const JS_TIMEOUT_MS = 5_000
export const JS_MAX_OUTPUT = 200_000

export interface OutChunk {
  stream: 'out' | 'err'
  text: string
}

export interface JsRunResult {
  chunks: OutChunk[]
  ok: boolean
  /** Son ifadenin biçimlenmiş değeri */
  result?: string
  /** Son ifadenin JSON'u (grafik için); serileştirilemezse yok */
  json?: string
  error?: string
  vars: VarInfo[]
  durationMs: number
  timedOut: boolean
  truncated: boolean
  /** Önsözün silemediği global'ler (boş olmalı) */
  hardeningGaps: string[]
}

/** Kaynağı blob: URL'li klasik bir Worker olarak başlatır. Blob worker oluşturan belgenin CSP'sini devralır. */
export function createIsolatedWorker(source: string): { worker: Worker; dispose: () => void } {
  const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }))
  const worker = new Worker(url, { name: 'g20-repl' })
  return {
    worker,
    dispose: () => {
      worker.terminate()
      URL.revokeObjectURL(url)
    },
  }
}

const isStr = (v: unknown): v is string => typeof v === 'string'

function readChunks(value: unknown): OutChunk[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((c: unknown) => {
    const chunk = c as { stream?: unknown; text?: unknown } | null
    return chunk && (chunk.stream === 'out' || chunk.stream === 'err') && isStr(chunk.text) ? [{ stream: chunk.stream, text: chunk.text }] : []
  })
}

function readVars(value: unknown): VarInfo[] {
  if (!Array.isArray(value)) return []
  return value.slice(0, 50).flatMap((v: unknown) => {
    const item = v as { name?: unknown; type?: unknown; preview?: unknown } | null
    return item && isStr(item.name) && isStr(item.type) && isStr(item.preview)
      ? [{ name: item.name.slice(0, 100), type: item.type.slice(0, 60), preview: item.preview.slice(0, 300) }]
      : []
  })
}

type Attempt = { kind: 'syntax'; message: string } | { kind: 'result'; result: JsRunResult }

function attempt(script: string, timeoutMs: number, maxOutput: number): Promise<Attempt> {
  return new Promise((resolve) => {
    const started = performance.now()
    const { worker, dispose } = createIsolatedWorker(script)
    const chunks: OutChunk[] = []
    let used = 0
    let truncated = false
    let running = false
    let gaps: string[] = []
    let settled = false

    const push = (chunk: OutChunk) => {
      if (used >= maxOutput) {
        truncated = true
        return
      }
      const text = chunk.text.slice(0, maxOutput - used)
      used += text.length
      if (text.length < chunk.text.length) truncated = true
      const last = chunks.at(-1)
      if (last && last.stream === chunk.stream) last.text += text
      else chunks.push({ stream: chunk.stream, text })
    }
    const finish = (outcome: Attempt) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      dispose()
      resolve(outcome)
    }
    const base = (): Omit<JsRunResult, 'ok'> => ({
      chunks,
      vars: [],
      durationMs: Math.round(performance.now() - started),
      timedOut: false,
      truncated,
      hardeningGaps: gaps,
    })
    const timer = setTimeout(() => finish({ kind: 'result', result: { ...base(), ok: false, timedOut: true, error: `Zaman aşımı: ${timeoutMs / 1000} sn sonra durduruldu` } }), timeoutMs)

    worker.onmessage = (event: MessageEvent<unknown>) => {
      const msg = event.data as { type?: unknown } & Record<string, unknown>
      if (!msg || typeof msg !== 'object') return
      if (msg.type === 'started') {
        running = true
        gaps = Array.isArray(msg.remaining) ? msg.remaining.filter(isStr).slice(0, 20) : []
      } else if (msg.type === 'out') {
        for (const chunk of readChunks(msg.chunks)) push(chunk)
        if (msg.truncated === true) truncated = true
      } else if (msg.type === 'done') {
        const result: JsRunResult = { ...base(), ok: msg.ok === true, vars: readVars(msg.vars) }
        if (isStr(msg.result)) result.result = msg.result.slice(0, maxOutput)
        if (isStr(msg.json) && msg.json.length <= 500_000) result.json = msg.json
        if (isStr(msg.error)) result.error = msg.error.slice(0, 2000)
        finish({ kind: 'result', result })
      }
    }
    worker.onerror = (event: ErrorEvent) => {
      event.preventDefault()
      const message = event.message || 'Worker hatası'
      // Önsöz 'started' göndermeden gelen hata = betik ayrıştırılamadı; hiçbir kod çalışmadı, başka kip denenebilir.
      if (!running) finish({ kind: 'syntax', message })
      else push({ stream: 'err', text: `${message}\n` })
    }
  })
}

/**
 * JS kodunu izole worker'da çalıştırır. Son ifade REPL gibi sonuç olarak döner; üst düzey await desteklenir.
 * Sözdizimi hatasında daha yalın kip denenir (ayrıştırma hatasında kod çalışmadığından yan etki tekrarlanmaz).
 */
export async function runJsInWorker(code: string, timeoutMs = JS_TIMEOUT_MS, maxOutput = JS_MAX_OUTPUT): Promise<JsRunResult> {
  let lastSyntax = 'Sözdizimi hatası'
  for (const mode of SCRIPT_MODES) {
    const outcome = await attempt(buildWorkerScript(code, mode, maxOutput), timeoutMs, maxOutput)
    if (outcome.kind === 'result') return outcome.result
    lastSyntax = outcome.message
  }
  return { chunks: [], ok: false, error: lastSyntax.replace(/^Uncaught /, ''), vars: [], durationMs: 0, timedOut: false, truncated: false, hardeningGaps: [] }
}
