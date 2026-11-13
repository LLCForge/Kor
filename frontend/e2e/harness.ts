// Desktop E2E harness (spec §16.4, §17.2, §17.3; ADR 0061).
//
// Starts the scenario's mock provider and isolated Kor home with the Rust helper
// (`cargo run -p kor-runtime --example mock_provider -- serve`), launches the real desktop app on it and
// attaches a driver:
// - Windows: Playwright over the WebView2 DevTools protocol (`--remote-debugging-port`), which also
//   gives screenshots for visual regression;
// - Linux / Windows without CDP: `tauri-driver` (W3C WebDriver) through `webdriver.ts`.
// macOS has neither (WKWebView exposes no automation protocol to tauri-driver or Playwright): the
// desktop E2E jobs run on Windows and Linux.
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import { mkdtempSync, mkdirSync, existsSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'

export const FRONTEND = resolve(fileURLToPath(import.meta.url), '../..')
export const REPO = resolve(FRONTEND, '..')
export const BACKEND = join(REPO, 'backend')
export const SCENARIOS = join(REPO, 'tests/e2e/scenarios')
/** CDP port for WebView2 (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`); the UI audit used 9333 before. */
export const CDP_PORT = Number(process.env.KOR_E2E_CDP_PORT ?? 9333)

export function desktopBinary(): string {
  const exe = process.platform === 'win32' ? 'kor-desktop.exe' : 'kor-desktop'
  const p = process.env.KOR_DESKTOP_BIN ?? join(FRONTEND, 'src-tauri/target/debug', exe)
  if (!existsSync(p)) throw new Error(`${p} missing: build it with \`npm run build:ui && cargo build\` in src-tauri, or set KOR_DESKTOP_BIN`)
  return p
}

export function korBinary(): string {
  const exe = process.platform === 'win32' ? 'kor.exe' : 'kor'
  return process.env.KOR_BIN ?? join(BACKEND, 'target/debug', exe)
}

/** The built helper (`cargo build -p kor-runtime --example mock_provider`), else `cargo run`. */
function helper(args: string[]): [string, string[]] {
  const exe = join(BACKEND, 'target/debug/examples', process.platform === 'win32' ? 'mock_provider.exe' : 'mock_provider')
  return existsSync(exe) ? [exe, args] : ['cargo', ['run', '-q', '-p', 'kor-runtime', '--example', 'mock_provider', '--', ...args]]
}

export interface Step { prompt?: string; approve?: 'yes' | 'no' | 'always'; command?: string; args?: string[]; desktop_args?: unknown[] }
export interface Scenario {
  name: string
  title: string
  surfaces: string[]
  steps: Step[]
  expect: { output: string[]; error: boolean }
}

export interface Served {
  url: string
  env: Record<string, string>
  project: string
  trace: string
  home: string
  scenario: Scenario
  root: string
  stop: () => Promise<void>
}

/** Mock provider + isolated home for one scenario; resolves once the helper printed its JSON line. */
export async function serve(scenarioFile: string): Promise<Served> {
  const root = mkdtempSync(join(tmpdir(), 'kor-e2e-desktop-'))
  const [cmd, args] = helper(['serve', '--scenario', scenarioFile, '--root', root, '--surface', 'desktop', '--kor', korBinary()])
  const proc = spawn(cmd, args, {
    cwd: BACKEND,
    stdio: ['pipe', 'pipe', 'inherit'],
    env: { ...process.env, CARGO_INCREMENTAL: '0' },
  })
  const line = await new Promise<string>((ok, fail) => {
    const rl = createInterface({ input: proc.stdout! })
    rl.once('line', ok)
    proc.once('exit', (c) => fail(new Error(`mock_provider exited (${c})`)))
  })
  const info = JSON.parse(line) as Omit<Served, 'root' | 'stop'>
  // One promise for every caller: a second `once('exit')` after the exit would wait forever.
  const exited = new Promise<void>((ok) => {
    if (proc.exitCode !== null || proc.signalCode !== null) ok()
    else proc.once('exit', () => ok())
  })
  return {
    ...info,
    root,
    stop: () => {
      proc.stdin!.end()
      return exited
    },
  }
}

/** The desktop app on the isolated home, with WebView2 remote debugging on `CDP_PORT` (Windows). */
export function launchDesktop(served: Served, extraEnv: Record<string, string> = {}): ChildProcess {
  const env: Record<string, string> = { ...(process.env as Record<string, string>), ...served.env, ...extraEnv }
  if (process.platform === 'win32') {
    env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = `--remote-debugging-port=${CDP_PORT}`
    // The app disables DevTools (and with it the debugging port) unless asked; debug builds only.
    env.KOR_DEVTOOLS = '1'
    // Tauri passes its own WebView2 arguments, which can shadow the variable above: the debug app
    // adds the port itself from this one.
    env.KOR_E2E_CDP_PORT = String(CDP_PORT)
  }
  const app = spawn(desktopBinary(), [], { env, stdio: ['ignore', 'ignore', 'pipe'], cwd: served.project })
  // The last of its stderr, for the error when it never becomes reachable (a crash at start).
  let tail = ''
  app.stderr?.on('data', (b: Buffer) => {
    tail = (tail + b.toString()).slice(-4000)
  })
  appLogs.set(app, () => tail)
  return app
}

const appLogs = new WeakMap<ChildProcess, () => string>()

/** Exit code and stderr tail of an app started by [`launchDesktop`], for error messages. */
export function appReport(app: ChildProcess): string {
  const exited = app.exitCode === null ? 'still running' : `exited with ${app.exitCode}`
  return `app ${exited}; stderr: ${appLogs.get(app)?.() || '(empty)'}`
}

/** Ends the app (and on Windows its WebView2 children) without leaving the debugging port open. */
export function closeDesktop(app: ChildProcess): void {
  if (app.pid === undefined) return
  if (process.platform === 'win32') spawnSync('taskkill', ['/PID', String(app.pid), '/T', '/F'], { stdio: 'ignore' })
  else app.kill('SIGTERM')
}

export interface Outcome { surface: string; events: unknown[]; problems: string[] }

/** The same checker the CLI and TUI runs use (`kor_runtime::e2e::check_with`). `as` names the result
 *  files: the side panel is checked against the desktop's trace (the desktop owns its sessions). */
export function check(served: Served, scenarioFile: string, output: string, as = 'desktop'): Outcome {
  const outFile = join(served.root, `output-${as}.txt`)
  writeFileSync(outFile, output)
  const [cmd, args] = helper(['check', '--scenario', scenarioFile, '--root', served.root, '--surface', 'desktop', '--output', outFile])
  const r = spawnSync(cmd, args, {
    cwd: BACKEND,
    encoding: 'utf8',
    env: { ...process.env, CARGO_INCREMENTAL: '0' },
  })
  if (!r.stdout.trim()) throw new Error(`checker failed: ${r.stderr}`)
  const outcome = JSON.parse(r.stdout) as Outcome
  const dir = join(FRONTEND, 'test-results/e2e')
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, `${served.scenario.name}-${as}.json`), JSON.stringify(outcome, null, 2))
  return outcome
}

/** CLI command → desktop bridge method, from parity.toml (`kor undo` → `core.undo`). */
export function desktopMethod(cli: string): string {
  const text = readFileSync(join(REPO, 'parity.toml'), 'utf8')
  for (const block of text.split('[[feature]]').slice(1)) {
    const get = (k: string) => new RegExp(`^${k} = "([^"]*)"`, 'm').exec(block)?.[1]
    if (get('cli') === cli && get('desktop') && get('desktop') !== '-') return get('desktop')!
  }
  throw new Error(`parity.toml has no desktop method for \`${cli}\``)
}
