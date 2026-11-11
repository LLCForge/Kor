import { spawn } from 'node:child_process'
import { decide, type PermissionConfig } from './permissions.ts'
import { detectProjectAt, verifyCommands } from './project-type.ts'
import { maskSecrets } from './secrets.ts'
import { loadG5Settings } from './settings-g5.ts'
import { buildEnv, currentShell, invocationFor, shellStateFor } from './shell-session.ts'

const VERIFY_TIMEOUT_MS = 10 * 60_000
const MAX_TAIL_CHARS = 4_000
const MAX_TOTAL_OUTPUT_CHARS = 200_000
/** Ajan düzeltemiyorsa sonsuz düzelt-doğrula döngüsüne girilmez. */
export const MAX_CONSECUTIVE_FAILURES = 3

export interface VerifyTarget {
  root: string
  permissions: PermissionConfig
  /** Kabuk durumunun bağlı olduğu nesne (oturumun ShellRegistry'si) */
  owner: object
  notify(text: string): void
  /** Turu durduran oturum kapanınca doğrulama da iptal edilir */
  signal: AbortSignal
}

interface VerifyState {
  running: boolean
  rerun: boolean
  failures: number
}

const states = new WeakMap<object, VerifyState>()

export interface CommandOutcome {
  code: number | null
  output: string
  timedOut: boolean
}

export function runVerifyCommand(command: string, target: VerifyTarget): Promise<CommandOutcome> {
  const shell = currentShell()
  const inv = invocationFor(shell, command)
  const state = shellStateFor(target.owner)
  const timeout = AbortSignal.timeout(VERIFY_TIMEOUT_MS)
  return new Promise((resolve) => {
    const child = spawn(inv.file, inv.args, {
      ...inv.options,
      cwd: target.root,
      env: buildEnv(state),
      signal: AbortSignal.any([target.signal, timeout]),
      windowsHide: true,
    })
    let output = ''
    const collect = (chunk: Buffer) => {
      output = (output + chunk.toString('utf8')).slice(-MAX_TOTAL_OUTPUT_CHARS)
    }
    child.stdout?.on('data', collect)
    child.stderr?.on('data', collect)
    child.on('error', (err) => {
      if (err.name !== 'AbortError') output += `\n[başlatılamadı: ${err.message}]`
    })
    child.on('close', (code) => resolve({ code, output, timedOut: timeout.aborted }))
  })
}

/**
 * Dosya değiştiren tur bitince algılanan typecheck/test komutlarını arka planda çalıştırır; başarısızsa çıktı
 * oturuma bildirim olarak gider. Ayar kapalıysa, komut yoksa veya bir deny kuralı/plan modu engelliyorsa hiçbir şey yapmaz.
 */
export async function autoVerify(
  target: VerifyTarget,
  isEnabled: () => Promise<boolean> = async () => (await loadG5Settings()).autoVerify,
): Promise<void> {
  const state = states.get(target.owner) ?? { running: false, rerun: false, failures: 0 }
  states.set(target.owner, state)
  if (state.failures >= MAX_CONSECUTIVE_FAILURES) return
  if (state.running) {
    state.rerun = true
    return
  }
  // Ayar/proje okunurken gelen ikinci çağrı da yeniden çalıştırma olarak işaretlensin.
  state.running = true
  try {
    if (!(await isEnabled())) return
    const commands = verifyCommands(await detectProjectAt(target.root)).filter(
      (cmd) => decide(target.permissions, 'bash', 'exec', cmd, new Set()).action !== 'deny',
    )
    if (commands.length === 0) return
    do {
      state.rerun = false
      let failed: { command: string; outcome: CommandOutcome } | null = null
      for (const command of commands) {
        const outcome = await runVerifyCommand(command, target)
        if (target.signal.aborted) return
        if (outcome.code !== 0 || outcome.timedOut) {
          failed = { command, outcome }
          break
        }
      }
      if (!failed) {
        state.failures = 0
        continue
      }
      state.failures++
      const tail = maskSecrets(failed.outcome.output.trim().slice(-MAX_TAIL_CHARS)) || '(çıktı yok)'
      const status = failed.outcome.timedOut ? 'zaman aşımı' : `çıkış kodu ${failed.outcome.code ?? '?'}`
      const stop =
        state.failures >= MAX_CONSECUTIVE_FAILURES
          ? `\nOtomatik doğrulama ${MAX_CONSECUTIVE_FAILURES} kez üst üste başarısız oldu ve bu sohbet için durduruldu; düzeltemiyorsan durumu kullanıcıya açıkla.`
          : ''
      target.notify(
        `<task-notification>\nOtomatik doğrulama (autoVerify) başarısız: ${failed.command} (${status}). Hataları düzelt ve tekrar doğrula.\nSon çıktı:\n${tail}${stop}\n</task-notification>`,
      )
      return
    } while (state.rerun)
  } finally {
    state.running = false
  }
}
