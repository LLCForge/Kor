import { execFile } from 'node:child_process'
import { access, readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { scrubEnv } from './secrets.ts'

export type DiagnosticTool = 'tsc' | 'eslint'

export interface Diagnostic {
  tool: DiagnosticTool
  /** Çalışma dizinine göre '/' ayraçlı yol; dizin dışındaysa veya genel hataysa ham metin */
  file: string
  /** false: tıklanınca açılamaz (dizin dışı veya dosyasız hata) */
  inWorkspace: boolean
  line: number
  column: number
  severity: 'error' | 'warning'
  code: string | null
  message: string
}

export interface ToolReport {
  tool: DiagnosticTool
  /** skipped: yapılandırma veya yerel kurulum yok; failed: çalıştırılamadı / çıktı okunamadı */
  status: 'ok' | 'skipped' | 'failed'
  reason: string | null
  diagnostics: Diagnostic[]
  truncated: boolean
  durationMs: number
}

export interface DiagnosticsReport {
  tsc: ToolReport
  eslint: ToolReport
}

const TIMEOUT_MS = 120_000
const MAX_OUTPUT_BYTES = 20_000_000
export const MAX_DIAGNOSTICS = 2000

const ESLINT_CONFIGS = [
  'eslint.config.js',
  'eslint.config.mjs',
  'eslint.config.cjs',
  'eslint.config.ts',
  'eslint.config.mts',
  'eslint.config.cts',
  '.eslintrc',
  '.eslintrc.js',
  '.eslintrc.cjs',
  '.eslintrc.json',
  '.eslintrc.yml',
  '.eslintrc.yaml',
]

const toPosix = (p: string) => p.split(path.sep).join('/')

function locate(root: string, file: string): { file: string; inWorkspace: boolean } {
  const rel = path.relative(root, path.resolve(root, file))
  if (rel === '' || rel.startsWith('..') || path.isAbsolute(rel)) return { file, inWorkspace: false }
  return { file: toPosix(rel), inWorkspace: true }
}

const TSC_LINE = /^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.*)$/
const TSC_GLOBAL = /^(error|warning) (TS\d+): (.*)$/

/** `tsc --pretty false` çıktısı; girintili devam satırları önceki mesaja eklenir. */
export function parseTscOutput(output: string, root: string): Diagnostic[] {
  const result: Diagnostic[] = []
  for (const raw of output.split(/\r?\n/)) {
    const line = raw.trimEnd()
    if (!line) continue
    const m = TSC_LINE.exec(line)
    if (m) {
      result.push({
        tool: 'tsc',
        ...locate(root, m[1] ?? ''),
        line: Number(m[2]),
        column: Number(m[3]),
        severity: m[4] === 'warning' ? 'warning' : 'error',
        code: m[5] ?? null,
        message: m[6] ?? '',
      })
      continue
    }
    const g = TSC_GLOBAL.exec(line)
    if (g) {
      result.push({
        tool: 'tsc',
        file: '',
        inWorkspace: false,
        line: 0,
        column: 0,
        severity: g[1] === 'warning' ? 'warning' : 'error',
        code: g[2] ?? null,
        message: g[3] ?? '',
      })
      continue
    }
    const last = result.at(-1)
    if (last && /^\s/.test(raw)) last.message += `\n${line.trim()}`
  }
  return result
}

const eslintSchema = z.array(
  z.object({
    filePath: z.string(),
    messages: z.array(
      z.object({
        ruleId: z.string().nullish(),
        severity: z.number(),
        message: z.string(),
        line: z.number().optional(),
        column: z.number().optional(),
      }),
    ),
  }),
)

/** `eslint --format json` çıktısı. Önünde başka metin varsa ilk '[' karakterinden okunur. */
export function parseEslintJson(output: string, root: string): Diagnostic[] {
  const start = output.indexOf('[')
  if (start < 0) throw new Error('ESLint çıktısı JSON değil')
  const files = eslintSchema.parse(JSON.parse(output.slice(start)))
  return files.flatMap((f) =>
    f.messages.map(
      (m): Diagnostic => ({
        tool: 'eslint',
        ...locate(root, f.filePath),
        line: m.line ?? 0,
        column: m.column ?? 0,
        severity: m.severity >= 2 ? 'error' : 'warning',
        code: m.ruleId ?? null,
        message: m.message,
      }),
    ),
  )
}

async function exists(file: string): Promise<boolean> {
  try {
    await access(file)
    return true
  } catch {
    return false
  }
}

const binSchema = z.object({ bin: z.union([z.string(), z.record(z.string(), z.string())]).optional() })

/** Projenin kendi node_modules içindeki komutun JS giriş dosyası; .cmd sarmalayıcısı kabuk gerektirdiği için kullanılmaz. */
export async function resolveLocalBin(root: string, pkg: string, bin: string): Promise<string | null> {
  const dir = path.join(root, 'node_modules', pkg)
  try {
    const manifest = binSchema.parse(JSON.parse(await readFile(path.join(dir, 'package.json'), 'utf8')))
    const rel = typeof manifest.bin === 'string' ? manifest.bin : manifest.bin?.[bin]
    if (!rel) return null
    const script = path.resolve(dir, rel)
    return (await exists(script)) ? script : null
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') console.warn(`${pkg} paket bilgisi okunamadı:`, err instanceof Error ? err.message : err)
    return null
  }
}

async function hasEslintConfig(root: string): Promise<boolean> {
  for (const name of ESLINT_CONFIGS) if (await exists(path.join(root, name))) return true
  try {
    const pkg: unknown = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'))
    return typeof pkg === 'object' && pkg !== null && 'eslintConfig' in pkg
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') console.warn('package.json okunamadı:', err instanceof Error ? err.message : err)
    return false
  }
}

/** Alt süreç ortamı: uygulamanın gizli anahtarı projedeki araçlara sızmaz. */
function childEnv(): NodeJS.ProcessEnv {
  return { ...scrubEnv(process.env), ELECTRON_RUN_AS_NODE: '1' }
}

interface RunResult {
  stdout: string
  stderr: string
  failure: string | null
}

/** Electron ana sürecinde process.execPath Electron'dur; ELECTRON_RUN_AS_NODE ile Node gibi çalışır. Kabuk kullanılmaz. */
function runScript(script: string, args: readonly string[], cwd: string): Promise<RunResult> {
  return new Promise((resolve) => {
    execFile(
      process.execPath,
      [script, ...args],
      { cwd, timeout: TIMEOUT_MS, maxBuffer: MAX_OUTPUT_BYTES, windowsHide: true, env: childEnv(), encoding: 'utf8' },
      (err, stdout, stderr) => {
        let failure: string | null = null
        if (err) {
          const e = err as NodeJS.ErrnoException & { killed?: boolean }
          if (e.killed) failure = `Zaman aşımı (${TIMEOUT_MS / 1000} sn)`
          else if (e.code === 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER') failure = 'Çıktı çok büyük'
          else if (typeof e.code === 'string') failure = `Başlatılamadı: ${e.message}`
          // Sayısal çıkış kodu: araç hata bulduğunda sıfırdan farklı döner, çıktı yine okunur.
        }
        resolve({ stdout, stderr, failure })
      },
    )
  })
}

function report(tool: DiagnosticTool, started: number, partial: Partial<ToolReport>): ToolReport {
  const diagnostics = partial.diagnostics ?? []
  return {
    tool,
    status: partial.status ?? 'ok',
    reason: partial.reason ?? null,
    diagnostics: diagnostics.slice(0, MAX_DIAGNOSTICS),
    truncated: diagnostics.length > MAX_DIAGNOSTICS,
    durationMs: Math.round(performance.now() - started),
  }
}

async function runTsc(root: string): Promise<ToolReport> {
  const started = performance.now()
  if (!(await exists(path.join(root, 'tsconfig.json')))) return report('tsc', started, { status: 'skipped', reason: 'tsconfig.json bulunamadı.' })
  const script = await resolveLocalBin(root, 'typescript', 'tsc')
  if (!script) return report('tsc', started, { status: 'skipped', reason: 'TypeScript bu projede kurulu değil (node_modules/typescript).' })
  const run = await runScript(script, ['--noEmit', '--pretty', 'false'], root)
  if (run.failure) return report('tsc', started, { status: 'failed', reason: run.failure })
  const diagnostics = parseTscOutput(run.stdout, root)
  if (diagnostics.length === 0 && run.stderr.trim()) return report('tsc', started, { status: 'failed', reason: run.stderr.trim().slice(0, 500) })
  return report('tsc', started, { diagnostics })
}

async function runEslint(root: string): Promise<ToolReport> {
  const started = performance.now()
  if (!(await hasEslintConfig(root))) return report('eslint', started, { status: 'skipped', reason: 'ESLint yapılandırması bulunamadı.' })
  const script = await resolveLocalBin(root, 'eslint', 'eslint')
  if (!script) return report('eslint', started, { status: 'skipped', reason: 'ESLint bu projede kurulu değil (node_modules/eslint).' })
  const run = await runScript(script, ['--format', 'json', '.'], root)
  if (run.failure) return report('eslint', started, { status: 'failed', reason: run.failure })
  try {
    return report('eslint', started, { diagnostics: parseEslintJson(run.stdout, root) })
  } catch (err) {
    console.warn('ESLint çıktısı ayrıştırılamadı:', err instanceof Error ? err.message : err)
    return report('eslint', started, { status: 'failed', reason: run.stderr.trim().slice(0, 500) || 'ESLint çıktısı okunamadı.' })
  }
}

export async function runDiagnostics(root: string): Promise<DiagnosticsReport> {
  const [tsc, eslint] = await Promise.all([runTsc(root), runEslint(root)])
  return { tsc, eslint }
}
