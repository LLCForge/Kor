import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { whenChildDone } from './kill-tree.ts'
import { scrubEnv } from './secrets.ts'
import { OutputDecoder } from './shell-session.ts'

/** kill_shell'in süreç gerçekten çıktı mı diye bekleyeceği süre. */
const STOP_WAIT_MS = 5_000
const MAX_BUFFER_CHARS = 1_000_000
const MAX_SHELLS = 8

/** shell: true ile başlayan süreç ağacını (cmd.exe + altındaki node vb.) tamamen sonlandırır. */
function killTree(child: ChildProcess): void {
  if (child.pid === undefined) return
  if (process.platform === 'win32') {
    spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true }).on('error', () => child.kill())
  } else {
    try {
      process.kill(-child.pid, 'SIGTERM') // detached: süreç grubu
    } catch {
      child.kill()
    }
  }
}

export interface BackgroundShell {
  id: string
  command: string
  startedAt: number
  status: 'running' | 'exited' | 'killed'
  exitCode: number | null
}

interface ShellState extends BackgroundShell {
  child: ChildProcess
  output: string
  /** bash_output'un son okuduğu konum */
  readOffset: number
  /** Tamponun başından atılan karakter sayısı (konumlar bunu hesaba katar) */
  dropped: number
  /** Çıktıyı tüketmeden izleyenler (monitor_shell) */
  listeners: Set<ShellListener>
}

export interface ShellListener {
  onData(chunk: string): void
  onExit(shell: BackgroundShell): void
}

/** Kabuk seçimi/kalıcı ortam için açık süreç tanımı; verilmezse shell: true kullanılır. */
export interface ShellSpawnSpec {
  file: string
  args: string[]
  options: SpawnOptions
  /** Süreç bitince çağrılır (geçici betik gibi). */
  cleanup?: () => void
}

/**
 * Oturuma bağlı arka plan komutları. Komutlar sınırsız süre çalışabilir (dev server gibi);
 * oturum kapanınca killAll ile hepsi sonlandırılır.
 */
export class ShellRegistry {
  readonly #shells = new Map<string, ShellState>()
  readonly #onExit: (shell: BackgroundShell, tail: string) => void

  constructor(onExit: (shell: BackgroundShell, tail: string) => void) {
    this.#onExit = onExit
  }

  start(command: string, cwd: string, spec?: ShellSpawnSpec): BackgroundShell {
    const running = [...this.#shells.values()].filter((s) => s.status === 'running').length
    if (running >= MAX_SHELLS) throw new Error(`Aynı anda en fazla ${MAX_SHELLS} arka plan komutu çalışabilir; önce birini kill_shell ile durdur.`)

    const detached = process.platform !== 'win32'
    const child = spec
      ? spawn(spec.file, spec.args, { cwd, ...spec.options, windowsHide: true, detached, stdio: ['ignore', 'pipe', 'pipe'] })
      : spawn(command, { cwd, shell: true, windowsHide: true, detached, stdio: ['ignore', 'pipe', 'pipe'], env: scrubEnv(process.env) })
    const state: ShellState = {
      id: randomUUID().slice(0, 8),
      command,
      startedAt: Date.now(),
      status: 'running',
      exitCode: null,
      child,
      output: '',
      readOffset: 0,
      dropped: 0,
      listeners: new Set(),
    }
    const append = (text: string) => {
      if (!text) return
      state.output += text
      for (const l of state.listeners) l.onData(text)
      // Uzun süre çalışan süreçler belleği doldurmasın: en eski çıktı atılır.
      if (state.output.length > MAX_BUFFER_CHARS) {
        const cut = state.output.length - MAX_BUFFER_CHARS
        state.output = state.output.slice(cut)
        state.dropped += cut
      }
    }
    const outDecoder = new OutputDecoder()
    const errDecoder = new OutputDecoder()
    child.stdout?.on('data', (chunk: Buffer) => append(outDecoder.write(chunk)))
    child.stderr?.on('data', (chunk: Buffer) => append(errDecoder.write(chunk)))
    child.on('error', (err) => append(`\n[başlatılamadı: ${err.message}]\n`))
    // Komut bitip arkasında boruyu tutan bir süreç bıraksa da kabuk "bitti" sayılır ve yuvası boşalır.
    whenChildDone(child, (code) => {
      append(outDecoder.end() + errDecoder.end())
      try {
        spec?.cleanup?.()
      } catch {
        // Arkada kalan süreç dosyayı tutuyor olabilir; geçici dizinde kalır.
      }
      if (state.status === 'running') state.status = 'exited'
      state.exitCode = code
      this.#onExit(this.#public(state), state.output.slice(-2000))
      for (const l of state.listeners) l.onExit(this.#public(state))
      state.listeners.clear()
    })
    this.#shells.set(state.id, state)
    return this.#public(state)
  }

  /** Son okumadan bu yana gelen çıktı. */
  read(id: string): { shell: BackgroundShell; output: string; truncated: boolean } {
    const state = this.#get(id)
    const start = Math.max(0, state.readOffset - state.dropped)
    const truncated = state.readOffset < state.dropped
    const output = state.output.slice(start)
    state.readOffset = state.dropped + state.output.length
    return { shell: this.#public(state), output, truncated }
  }

  /** Çıktıyı okuma konumunu değiştirmeden izler; dönen fonksiyon aboneliği bitirir. tail: mevcut tamponun sonu. */
  subscribe(id: string, listener: ShellListener): { shell: BackgroundShell; tail: string; unsubscribe: () => void } {
    const state = this.#get(id)
    if (state.status === 'running') state.listeners.add(listener)
    return { shell: this.#public(state), tail: state.output.slice(-65_536), unsubscribe: () => state.listeners.delete(listener) }
  }

  kill(id: string): BackgroundShell {
    const state = this.#get(id)
    if (state.status === 'running') {
      state.status = 'killed'
      killTree(state.child)
    }
    return this.#public(state)
  }

  /** kill_shell için: ağacı öldürür ve sürecin gerçekten çıkmasını bekler. stopped false ise bu çağrı onu durdurmadı. */
  async stop(id: string, waitMs = STOP_WAIT_MS): Promise<{ shell: BackgroundShell; stopped: boolean }> {
    const state = this.#get(id)
    if (state.status !== 'running') return { shell: this.#public(state), stopped: false }
    const child = state.child
    const exited = new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => resolve(false), waitMs)
      child.once('exit', () => {
        clearTimeout(timer)
        resolve(true)
      })
    })
    state.status = 'killed'
    killTree(child)
    const stopped = child.exitCode !== null || child.signalCode !== null || (await exited)
    if (!stopped) state.status = 'running'
    return { shell: this.#public(state), stopped }
  }

  killAll(): void {
    for (const s of this.#shells.values()) {
      if (s.status === 'running') {
        s.status = 'killed'
        killTree(s.child)
      }
    }
  }

  list(): BackgroundShell[] {
    return [...this.#shells.values()].map((s) => this.#public(s))
  }

  #get(id: string): ShellState {
    const state = this.#shells.get(id)
    if (!state) throw new Error(`Arka plan komutu bulunamadı: ${id}`)
    return state
  }

  #public(s: ShellState): BackgroundShell {
    return { id: s.id, command: s.command, startedAt: s.startedAt, status: s.status, exitCode: s.exitCode }
  }
}
