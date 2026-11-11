// [g27] Sürüm yoklaması ve winget kurulumu. Kurulum yalnızca Kur çağrısıyla başlar.
import { execFile } from 'node:child_process'
import { z } from 'zod'
import { RUNTIMES, readVersion, runtimeById, tickFromLine, type InstallTick, type RuntimeSpec } from '../core/runtimes-g27.ts'
import { CHANNELS } from './ipc.ts'

export interface RuntimeStatus {
  id: string
  name: string
  installed: boolean
  version: string | null
  winget: boolean
}

const ids = RUNTIMES.map((item) => item.id) as [string, ...string[]]
const idSchema = z.enum(ids)

function run(file: string, args: readonly string[], timeoutMs: number): Promise<{ code: number | null; text: string }> {
  return new Promise((resolve) => {
    execFile(file, [...args], { timeout: timeoutMs, windowsHide: true, encoding: 'utf8', maxBuffer: 1024 * 1024 }, (err, stdout, stderr) => {
      const code = err && 'code' in err && typeof err.code === 'number' ? err.code : err ? 1 : 0
      resolve({ code, text: `${stdout ?? ''}\n${stderr ?? ''}` })
    })
  })
}

async function wingetPresent(): Promise<boolean> {
  if (process.platform !== 'win32') return false
  const result = await run('winget', ['--version'], 8000)
  return result.code === 0 && /v?\d+\.\d+/.test(result.text)
}

async function probe(spec: RuntimeSpec): Promise<string | null> {
  for (const [file, args] of spec.probes) {
    const result = await run(file, args, 8000)
    const version = readVersion(spec, result.text)
    if (version) return version
  }
  return null
}

export async function listRuntimes(): Promise<RuntimeStatus[]> {
  const winget = await wingetPresent()
  const rows = await Promise.all(
    RUNTIMES.map(async (spec) => {
      const version = await probe(spec)
      return { id: spec.id, name: spec.name, installed: version !== null, version, winget }
    }),
  )
  return rows
}

let busy = false

export function registerRuntimes(
  handle: <A extends unknown[], R>(channel: string, schema: z.ZodType<A>, fn: (...args: A) => Promise<R> | R) => void,
  send: (tick: InstallTick & { id: string }) => void,
  openPage: (url: string) => Promise<void>,
): void {
  handle(CHANNELS.g27ListRuntimes, z.tuple([]), () => listRuntimes())
  handle(CHANNELS.g27InstallRuntime, z.tuple([idSchema]), async (id: string) => {
    const spec = runtimeById(id)
    if (!spec) throw new Error('Bilinmeyen araç')
    if (busy) throw new Error('Başka bir kurulum sürüyor')
    const existing = await probe(spec)
    if (existing) return { installed: true, version: existing, opened: false }
    if (process.platform !== 'win32' || !(await wingetPresent())) {
      await openPage(spec.page)
      return { installed: false, version: null, opened: true }
    }
    busy = true
    send({ id, phase: 'indiriliyor', percent: null, line: 'winget başlatıldı' })
    try {
      const outcome = await new Promise<{ code: number | null; text: string }>((resolve) => {
        const child = execFile(
          'winget',
          ['install', '--id', spec.wingetId, '--exact', '--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity'],
          { windowsHide: true, timeout: 20 * 60_000, maxBuffer: 4 * 1024 * 1024 },
          (err, stdout, stderr) => {
            const code = err && 'code' in err && typeof err.code === 'number' ? err.code : err ? 1 : 0
            resolve({ code, text: `${stdout ?? ''}\n${stderr ?? ''}` })
          },
        )
        const onData = (chunk: Buffer) => {
          for (const line of chunk.toString('utf8').split(/\r?\n/)) {
            if (!line.trim()) continue
            const tick = tickFromLine(line)
            send({ id, ...tick })
          }
        }
        child.stdout?.on('data', onData)
        child.stderr?.on('data', onData)
      })
      const version = await probe(spec)
      if (outcome.code === 0 || version) {
        send({ id, phase: 'bitti', percent: 100, line: version ? `Kuruldu: ${version}` : 'Kurulum bitti' })
        return { installed: version !== null || outcome.code === 0, version, opened: false }
      }
      const last = outcome.text.trim().split(/\r?\n/).at(-1) ?? 'Kurulum başarısız'
      send({ id, phase: 'hata', percent: null, line: last.slice(0, 240) })
      throw new Error(last.slice(0, 240))
    } finally {
      busy = false
    }
  })
}
