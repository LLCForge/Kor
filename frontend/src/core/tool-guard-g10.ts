// [g10] Araç yürütme koruması: devre kesici, takılan araç bekçisi, okuma araçlarında geçici hata yeniden denemesi,
// çıktı şema doğrulaması, düzenleme çakışması tespiti, riskli komut öncesi otomatik kontrol noktası ve atomik yazma.
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { CircuitBreaker, isTransientError, retryWithBackoff, sleep, type G10Event, type RetryBudget } from './resilience-g10.ts'
import { maskSecrets } from './secrets.ts'
import { reportSnapshotChanges, riskyPlan, takeSnapshot, type Snapshot } from './snapshot-g10.ts'
import { resolveInWorkspace, type Tool, type ToolContext } from './tools.ts'

export const DEFAULT_TOOL_TIMEOUT_MS = 300_000
// Kullanıcıyı, alt ajanı veya kendi süre sınırı olan bir izlemeyi bekleyen araçlar bekçiye tabi değildir.
const WATCHDOG_EXEMPT = new Set(['agent', 'run_team', 'ask_user', 'exit_plan_mode', 'monitor_shell', 'start_session', 'send_session_message'])
/** 'read' türünde ama alt ajan çalıştırır: yeniden deneme dosya düzenlemelerini iki kez uygular. */
const NEVER_RETRY = new Set(['agent', 'run_team'])
const READ_TRACKED = new Set(['read_file', 'read_notebook'])
/** Tamamen üzerine yazan / konuma göre değiştiren araçlar: dışarıdan değişmiş dosyada engellenir. */
const WRITE_BLOCKING = new Set(['write_file', 'edit_notebook'])
/** old_string'e bağlı düzenlemeler güncel içeriğe uygulanır; yalnızca not düşülür. */
const WRITE_NOTING = new Set(['edit_file', 'multi_edit'])
const MAX_RESULT_CHARS = 400_000
const MAX_TRACKED_FILES = 5_000
const BREAKER = { failureThreshold: 4, cooldownMs: 60_000 }

export interface GuardEnv {
  emit(event: G10Event): void
  budget: RetryBudget
  toolTimeoutMs: number
  /** Kurtarma günlüğü: son model adımından sonra tamamlanan yan etkili araç */
  noteApplied(summary: string): void
}

export class ToolTimeoutError extends Error {
  readonly reason: string
  constructor(tool: string, ms: number, reason: string) {
    super(`"${tool}" aracı ${Math.round(ms / 1000)} sn içinde bitmedi ve iptal edildi. Olası neden: ${reason}`)
    this.name = 'ToolTimeoutError'
    this.reason = reason
  }
}

function stuckReason(tool: string): string {
  if (tool === 'bash') return 'komut etkileşimli girdi bekliyor veya çok uzun sürüyor; uzun işler için run_in_background kullan.'
  if (tool.startsWith('web_')) return 'uzak sunucu yanıt vermiyor.'
  if (tool === 'code_intel') return 'dil sunucusu yanıt vermiyor.'
  return 'araç yanıt vermedi (kilitli dosya, ağ veya sonsuz döngü olabilir).'
}

const breakers = new Map<string, CircuitBreaker>()

export function toolBreaker(name: string): CircuitBreaker {
  let b = breakers.get(name)
  if (!b) {
    b = new CircuitBreaker(name, BREAKER)
    breakers.set(name, b)
  }
  return b
}

// ---------- Düzenleme çakışması (hash) ----------

const fileHashes = new Map<string, string | null>()

async function fileHash(abs: string): Promise<string | null> {
  try {
    return createHash('sha256').update(await readFile(abs)).digest('hex')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

async function rememberHash(abs: string): Promise<void> {
  try {
    const h = await fileHash(abs)
    fileHashes.delete(abs)
    fileHashes.set(abs, h)
    if (fileHashes.size > MAX_TRACKED_FILES) fileHashes.delete(fileHashes.keys().next().value ?? '')
  } catch (err) {
    console.warn('Dosya özeti alınamadı:', err instanceof Error ? err.message : err)
  }
}

/** Ajan bu dosyayı en son okuduğu/yazdığından beri dışarıda değişti mi (bilinmiyorsa false). */
export async function changedSinceSeen(abs: string): Promise<boolean> {
  const known = fileHashes.get(abs)
  return known !== undefined && (await fileHash(abs)) !== known
}

const pathArgs = z.object({ path: z.string().min(1).max(1024) })

function targetOf(raw: unknown, root: string): { abs: string; rel: string } | null {
  const parsed = pathArgs.safeParse(raw)
  if (!parsed.success) return null
  try {
    return { abs: resolveInWorkspace(root, parsed.data.path), rel: parsed.data.path }
  } catch {
    return null // araç kendi doğrulamasında anlamlı hatayı verir
  }
}

// ---------- Koruma ----------

const outputSchema = z.string()

/** Araç sonucu metin olmalı; NUL karakterleri ve aşırı uzunluk model isteğini bozmasın. */
export function validateToolOutput(tool: string, value: unknown): string {
  const parsed = outputSchema.safeParse(value)
  if (!parsed.success) throw new Error(`"${tool}" aracı geçersiz çıktı döndürdü (beklenen metin, gelen ${value === null ? 'null' : typeof value}).`)
  const clean = parsed.data.replaceAll('\0', '�')
  return clean.length > MAX_RESULT_CHARS ? `${clean.slice(0, MAX_RESULT_CHARS)}\n[g10: çıktı ${clean.length - MAX_RESULT_CHARS} karakter kesildi]` : clean
}

async function withWatchdog(tool: Tool, inner: Tool['run'], raw: unknown, ctx: ToolContext, env: GuardEnv): Promise<unknown> {
  const ms = env.toolTimeoutMs
  if (ms <= 0 || WATCHDOG_EXEMPT.has(tool.name)) return inner(raw, ctx)
  const controller = new AbortController()
  const run = inner(raw, { ...ctx, signal: AbortSignal.any([ctx.signal, controller.signal]) })
  let timer: NodeJS.Timeout | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new ToolTimeoutError(tool.name, ms, stuckReason(tool.name))), ms)
  })
  // Sinyali yok sayan araçta da Durdur hemen etkili olsun (araç arka planda bitebilir).
  let onAbort = (): void => undefined
  const aborted = new Promise<never>((_, reject) => {
    onAbort = () => reject(ctx.signal.reason as Error)
    if (ctx.signal.aborted) onAbort()
    else ctx.signal.addEventListener('abort', onAbort, { once: true })
  })
  try {
    return await Promise.race([run, timeout, aborted])
  } catch (err) {
    if (err instanceof ToolTimeoutError) {
      controller.abort(err)
      const stopped = await Promise.race([run.then(() => true, () => true), sleep(2000).then(() => false)])
      err.message += stopped ? ' Araç iptal sinyaline uyup durdu.' : ' Araç iptal sinyaline 2 sn içinde yanıt vermedi; arka planda sürüyor olabilir.'
      env.emit({ kind: 'toolTimeout', tool: tool.name, seconds: Math.round(ms / 1000), reason: err.reason })
    }
    throw err
  } finally {
    clearTimeout(timer)
    ctx.signal.removeEventListener('abort', onAbort)
  }
}

const bashArgs = z.object({ command: z.string(), run_in_background: z.boolean().optional() })

async function snapshotFor(tool: Tool, raw: unknown, ctx: ToolContext): Promise<Snapshot | null> {
  if (tool.name !== 'bash' || !ctx.onFileChange) return null
  const parsed = bashArgs.safeParse(raw)
  // Arka plan komutunun etkisi sonra gerçekleşir; karşılaştırma yapılamaz.
  if (!parsed.success || parsed.data.run_in_background) return null
  const plan = riskyPlan(parsed.data.command)
  if (!plan) return null
  try {
    return await takeSnapshot(ctx.root, plan)
  } catch (err) {
    console.warn('Otomatik kontrol noktası alınamadı:', err instanceof Error ? err.message : err)
    return null
  }
}

async function reportSnapshot(snap: Snapshot, ctx: ToolContext, env: GuardEnv): Promise<number> {
  const onFileChange = ctx.onFileChange
  if (!onFileChange) return 0
  try {
    const n = await reportSnapshotChanges(snap, onFileChange)
    if (n > 0 || snap.partial) env.emit({ kind: 'checkpoint', files: n, partial: snap.partial })
    return n
  } catch (err) {
    console.warn('Kontrol noktası karşılaştırması başarısız:', err instanceof Error ? err.message : err)
    return 0
  }
}

function looksFailed(result: string): boolean {
  return /^(Hata|Engellendi|Kullanıcı bu işlemi reddetti|Komut başlatılamadı|Döngü)/.test(result)
}

async function guardedRun(tool: Tool, inner: Tool['run'], raw: unknown, ctx: ToolContext, env: GuardEnv): Promise<string> {
  const breaker = toolBreaker(tool.name)
  breaker.assertPass()
  const target = READ_TRACKED.has(tool.name) || WRITE_BLOCKING.has(tool.name) || WRITE_NOTING.has(tool.name) ? targetOf(raw, ctx.root) : null
  let conflictNote = ''
  if (target && (WRITE_BLOCKING.has(tool.name) || WRITE_NOTING.has(tool.name)) && (await changedSinceSeen(target.abs))) {
    env.emit({ kind: 'conflict', path: target.rel })
    if (WRITE_BLOCKING.has(tool.name)) {
      breaker.release()
      throw new Error(
        `Çakışma: ${target.rel} son okunmasından/yazılmasından sonra dışarıda değiştirildi (ör. editörde kaydedildi). ` +
          'Kullanıcının değişikliğini ezmemek için yazılmadı; önce read_file ile güncel halini oku, sonra değişikliği yeniden uygula.',
      )
    }
    conflictNote = `\n[Not: ${target.rel} son okunmasından sonra dışarıda değişmişti; düzenleme güncel içeriğe uygulandı. Gerekirse dosyayı yeniden oku.]`
  }
  const snap = await snapshotFor(tool, raw, ctx)

  let output: string
  try {
    const attempt = () => withWatchdog(tool, inner, raw, ctx, env)
    // Yalnızca okuma araçları tekrarlanır: yan etkili bir işlemi tekrar çalıştırmak çift uygulama demektir.
    // Bekçi zaman aşımı tekrarlanmaz: takılan araç her denemede yine tam süre bekletirdi.
    const retryable = (err: unknown) => isTransientError(err) && !(err instanceof ToolTimeoutError)
    const result =
      tool.kind === 'read' && !NEVER_RETRY.has(tool.name)
        ? await retryWithBackoff(attempt, { retries: 2, baseMs: 250, maxMs: 2_000, isRetryable: retryable, signal: ctx.signal, budget: env.budget })
        : await attempt()
    output = validateToolOutput(tool.name, result)
  } catch (err) {
    if (isTransientError(err) || (err instanceof Error && err.message.includes('geçersiz çıktı döndürdü'))) {
      if (breaker.failure()) env.emit({ kind: 'circuitOpen', target: tool.name, retryInSec: Math.ceil(breaker.retryInMs() / 1000) })
    } else {
      breaker.release()
    }
    if (snap) await reportSnapshot(snap, ctx, env)
    if (target && READ_TRACKED.has(tool.name)) await rememberHash(target.abs)
    throw err
  }
  breaker.success()
  if (target) await rememberHash(target.abs)
  if (snap) {
    const n = await reportSnapshot(snap, ctx, env)
    if (n > 0) output += `\n[Otomatik kontrol noktası: ${n} dosyanın önceki hali kaydedildi; "Buraya geri sar" ile geri alınabilir.]`
  }
  if (tool.kind !== 'read' && !looksFailed(output)) env.noteApplied(`${tool.name}: ${maskSecrets(tool.summarize(raw)).slice(0, 200)}`)
  return output + conflictNote
}

const guarded = new WeakSet<Tool>()

/**
 * Oturumun araç nesnelerini yerinde sarar (dizi kimliği değişmez; g5'in döngü koruması yeniden sarmaz).
 * Paylaşılan çekirdek araç nesnelerine (globals) dokunulmaz.
 */
export function guardTools(tools: readonly Tool[], globals: readonly Tool[], envFor: (ctx: ToolContext) => GuardEnv): void {
  for (const t of tools) {
    if (guarded.has(t) || globals.includes(t)) continue
    const inner = t.run.bind(t)
    t.run = (raw, ctx) => guardedRun(t, inner, raw, ctx, envFor(ctx))
    guarded.add(t)
  }
}
