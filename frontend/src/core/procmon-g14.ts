// [g14] Alt süreç kaynak izleme: uygulamanın alt süreç ağacının (arka plan kabukları, MCP vb.) CPU ve belleği.
// Windows'ta PowerShell + WMI (Get-CimInstance Win32_Process), diğerlerinde ps. Kabuksuz argüman dizisi,
// sabit betik (kullanıcı girdisi yok), zaman aşımı ve çıktı sınırı ile çalışır.
import { execFile } from 'node:child_process'
import os from 'node:os'
import { z } from 'zod'
import { maskSecrets } from './secrets.ts'

export interface ProcRow {
  pid: number
  ppid: number
  name: string
  commandLine: string
  memBytes: number
  /** Toplam CPU süresi (ms); yalnızca Windows'ta. Yüzde iki örnek arasındaki farktan hesaplanır. */
  cpuMs: number | null
  /** ps'in verdiği ömür boyu ortalama CPU yüzdesi (Windows dışı) */
  pcpu: number | null
}

const WIN_SCRIPT =
  '[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-CimInstance -ClassName Win32_Process | ' +
  'Select-Object ProcessId,ParentProcessId,Name,CommandLine,WorkingSetSize,KernelModeTime,UserModeTime | ConvertTo-Json -Compress'

const num = z.number().finite().nullish()
const winRow = z.object({
  ProcessId: z.number().int(),
  ParentProcessId: z.number().int().nullish(),
  Name: z.string().nullish(),
  CommandLine: z.string().nullish(),
  WorkingSetSize: num,
  KernelModeTime: num,
  UserModeTime: num,
})

/** ConvertTo-Json tek süreçte nesne, çokta dizi döner. Süre birimi 100 ns. */
export function parseWin32Processes(json: string): ProcRow[] {
  const raw: unknown = JSON.parse(json)
  const rows = z.array(winRow).parse(Array.isArray(raw) ? raw : [raw])
  return rows.map((r) => ({
    pid: r.ProcessId,
    ppid: r.ParentProcessId ?? 0,
    name: r.Name ?? '?',
    commandLine: r.CommandLine ?? '',
    memBytes: r.WorkingSetSize ?? 0,
    cpuMs: ((r.KernelModeTime ?? 0) + (r.UserModeTime ?? 0)) / 10_000,
    pcpu: null,
  }))
}

/** `ps -A -o pid=,ppid=,rss=,pcpu=,args=` çıktısı (rss KiB). */
export function parsePsOutput(text: string): ProcRow[] {
  const rows: ProcRow[] = []
  for (const line of text.split('\n')) {
    const m = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+([\d.]+)\s+(.*)$/.exec(line)
    if (!m) continue
    const args = m[5] ?? ''
    rows.push({
      pid: Number(m[1]),
      ppid: Number(m[2]),
      name: (args.split(/\s+/)[0] ?? '').split('/').at(-1) ?? '?',
      commandLine: args,
      memBytes: Number(m[3]) * 1024,
      cpuMs: null,
      pcpu: Number(m[4]),
    })
  }
  return rows
}

export interface ProcessGroup {
  /** Arka plan kabuğu kimliği (ShellRegistry) eşleştiyse */
  shellId: string | null
  label: string
  rootPid: number
  pids: number[]
  memBytes: number
  /** İlk örnekte (karşılaştırılacak önceki ölçüm yok) null */
  cpuPercent: number | null
}

export interface ShellRef {
  id: string
  command: string
}

export interface CpuSample {
  at: number
  cpuMs: Map<number, number>
}

function subtree(rows: readonly ProcRow[], rootPid: number): ProcRow[] {
  const children = new Map<number, ProcRow[]>()
  for (const r of rows) children.set(r.ppid, [...(children.get(r.ppid) ?? []), r])
  const out: ProcRow[] = []
  const stack = [rootPid]
  const seen = new Set<number>()
  while (stack.length > 0) {
    const pid = stack.pop() ?? 0
    for (const child of children.get(pid) ?? []) {
      // Windows pid yeniden kullanır; döngü koruması.
      if (seen.has(child.pid) || child.pid === rootPid) continue
      seen.add(child.pid)
      out.push(child)
      stack.push(child.pid)
    }
  }
  return out
}

/**
 * Ana sürecin doğrudan alt süreçlerini (Electron'un kendi yardımcı süreçleri hariç) alt ağaçlarıyla gruplar.
 * Arka plan kabuğu, komut satırı kabuğun komutunu içeren alt süreçle eşleştirilir.
 */
export function groupProcesses(
  rows: readonly ProcRow[],
  rootPid: number,
  shells: readonly ShellRef[],
  isOwnHelper: (row: ProcRow) => boolean,
  prev: CpuSample | null,
  now: number,
  cpuCount: number,
): ProcessGroup[] {
  const direct = rows.filter((r) => r.ppid === rootPid && !isOwnHelper(r))
  const unmatched = [...shells]
  return direct.map((root) => {
    const members = [root, ...subtree(rows, root.pid)]
    const idx = unmatched.findIndex((s) => s.command.trim().length > 0 && root.commandLine.includes(s.command.trim()))
    const shell = idx >= 0 ? unmatched.splice(idx, 1)[0] : undefined
    let cpuPercent: number | null = null
    if (members.every((m) => m.cpuMs !== null)) {
      const elapsed = prev ? now - prev.at : 0
      const delta = members.reduce((s, m) => s + Math.max(0, (m.cpuMs ?? 0) - (prev?.cpuMs.get(m.pid) ?? m.cpuMs ?? 0)), 0)
      cpuPercent = prev && elapsed > 0 ? Math.min(100, (delta / (elapsed * Math.max(1, cpuCount))) * 100) : null
    } else {
      cpuPercent = members.reduce((s, m) => s + (m.pcpu ?? 0), 0) / Math.max(1, cpuCount)
    }
    return {
      shellId: shell?.id ?? null,
      // Eşleşmeyen süreçlerin komut satırı gösterilmez (gizli argüman taşıyabilir); yalnızca adı.
      label: shell ? maskSecrets(shell.command).slice(0, 200) : root.name,
      rootPid: root.pid,
      pids: members.map((m) => m.pid),
      memBytes: members.reduce((s, m) => s + m.memBytes, 0),
      cpuPercent,
    }
  })
}

export function cpuSampleOf(rows: readonly ProcRow[], at: number): CpuSample {
  return { at, cpuMs: new Map(rows.flatMap((r) => (r.cpuMs === null ? [] : [[r.pid, r.cpuMs] as const]))) }
}

/** Süreç tablosunu okur. Başarısızlıkta hata fırlatır (çağıran kullanıcıya genel mesaj gösterir). */
export function readProcessTable(timeoutMs = 10_000): Promise<ProcRow[]> {
  const win = process.platform === 'win32'
  const file = win ? 'powershell.exe' : 'ps'
  const args = win ? ['-NoProfile', '-NonInteractive', '-Command', WIN_SCRIPT] : ['-A', '-o', 'pid=,ppid=,rss=,pcpu=,args=']
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout: timeoutMs, windowsHide: true, maxBuffer: 32 * 1024 * 1024, encoding: 'utf8' }, (err, stdout) => {
      if (err) return reject(new Error(`Süreç listesi alınamadı: ${err.message}`))
      try {
        resolve(win ? parseWin32Processes(stdout) : parsePsOutput(stdout))
      } catch (parseErr) {
        reject(new Error(`Süreç listesi çözümlenemedi: ${parseErr instanceof Error ? parseErr.message : String(parseErr)}`))
      }
    })
  })
}

export const cpuCount = () => os.availableParallelism()
