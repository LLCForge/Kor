// [g3] Kod bloğundaki kabuk komutunu kullanıcı onayından sonra çalıştırma.
import { spawn } from 'node:child_process'
import { isPowerShellLanguage } from './code-langs.ts'
import { scrubEnv } from './secrets.ts'

export const SNIPPET_TIMEOUT_MS = 60_000
const MAX_OUTPUT_CHARS = 100_000

export interface SnippetResult {
  output: string
  exitCode: number | null
  timedOut: boolean
  truncated: boolean
}

/** PowerShell blokları powershell/pwsh ile, diğerleri bash aracıyla aynı sistem kabuğuyla çalışır. */
export function runSnippet(code: string, language: string, cwd: string, timeoutMs = SNIPPET_TIMEOUT_MS): Promise<SnippetResult> {
  const timeout = AbortSignal.timeout(timeoutMs)
  const ps = isPowerShellLanguage(language)
  const [command, args, shell] = ps
    ? [process.platform === 'win32' ? 'powershell.exe' : 'pwsh', ['-NoProfile', '-NonInteractive', '-Command', code], false]
    : [code, [], true]
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, shell, signal: timeout, windowsHide: true, env: scrubEnv(process.env) })
    let output = ''
    let truncated = false
    const collect = (chunk: Buffer) => {
      if (output.length >= MAX_OUTPUT_CHARS) {
        truncated = true
        return
      }
      output += chunk.toString('utf8')
    }
    child.stdout.on('data', collect)
    child.stderr.on('data', collect)
    child.on('error', (err) => {
      // Zaman aşımında süreç öldürülür; sonucu 'close' kısmi çıktıyla bildirir.
      if (err.name !== 'AbortError') reject(err)
    })
    child.on('close', (exitCode) => {
      resolve({ output: output.slice(0, MAX_OUTPUT_CHARS), exitCode, timedOut: timeout.aborted, truncated: truncated || output.length > MAX_OUTPUT_CHARS })
    })
  })
}
