// [g10] Riskli kabuk komutlarından (toplu silme, taşıma, yeniden adlandırma, yıkıcı git, > ile üzerine yazma) önce
// etkilenecek dosyaların içeriği saklanır; komuttan sonra değişenler onFileChange ile bildirilir. Böylece g4'ün
// tur kontrol noktasına ve "Buraya geri sar" akışına, yazma araçları dışındaki değişiklikler de girer.
import { execFile } from 'node:child_process'
import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { gitExePath } from './git-path.ts'
import { isSensitivePath } from './permissions.ts'
import { confineToWorkspace } from './workspace.ts'

export const SNAPSHOT_MAX_FILES = 500
export const SNAPSHOT_MAX_CHARS = 20_000_000
const MAX_FILE_BYTES = 5_000_000
const SKIP_DIRS = new Set(['node_modules', '.git'])

const PATH_COMMANDS = new Set([
  'rm', 'del', 'erase', 'rmdir', 'rd', 'unlink', 'shred', 'remove-item', 'ri',
  'mv', 'move', 'move-item', 'mi', 'ren', 'rename', 'rename-item', 'rni',
])
const GIT_DESTRUCTIVE = new Set(['rm', 'mv', 'clean', 'reset', 'checkout', 'restore', 'stash', 'switch'])

export interface RiskyPlan {
  /** Komutta geçen yol argümanları (çalışma dizinine göre veya mutlak) */
  paths: string[]
  /** git'in yıkıcı alt komutu: değişmiş/izlenmeyen tüm dosyalar saklanır */
  git: boolean
}

function tokenize(segment: string): string[] {
  return [...segment.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map((m) => m[1] ?? m[2] ?? m[3] ?? '')
}

/** Riskli değilse null. Ayrıştırma bilinçli olarak basittir: yanlış pozitif yalnızca fazladan bir kopya demektir. */
export function riskyPlan(command: string): RiskyPlan | null {
  const plan: RiskyPlan = { paths: [], git: false }
  for (const segment of command.split(/&&|\|\||[;\n|&]/)) {
    const tokens = tokenize(segment.trim())
    const head = (tokens[0] ?? '').toLowerCase().split(/[\\/]/).pop()?.replace(/\.exe$/, '') ?? ''
    if (PATH_COMMANDS.has(head)) {
      plan.paths.push(...tokens.slice(1).filter((t) => t && !t.startsWith('-')))
    } else if (head === 'git') {
      const sub = tokens.slice(1).find((t) => !t.startsWith('-'))?.toLowerCase() ?? ''
      if (GIT_DESTRUCTIVE.has(sub)) plan.git = true
    }
  }
  // "> dosya" tamamen üzerine yazar; ">>" eklemedir ve "2>&1" yönlendirmedir.
  for (const m of command.matchAll(/(?:^|[^>&\d])>\s*("[^"]+"|'[^']+'|[^\s;&|>]+)/g)) {
    const target = (m[1] ?? '').replace(/^["']|["']$/g, '')
    if (target && !target.startsWith('&') && !/^(nul|\/dev\/null|\$null)$/i.test(target)) plan.paths.push(target)
  }
  return plan.paths.length > 0 || plan.git ? plan : null
}

export interface Snapshot {
  root: string
  /** Göreli yol → komuttan önceki içerik (null: dosya yoktu) */
  files: Map<string, string | null>
  /** Komuttan sonra yeni dosya aranacak klasörler/yollar (mutlak) */
  watch: string[]
  partial: boolean
  skippedBinary: number
}

const toPosix = (p: string) => p.split(path.sep).join('/')

async function readText(abs: string): Promise<string | null | 'binary'> {
  try {
    const info = await stat(abs)
    if (!info.isFile() || info.size > MAX_FILE_BYTES) return 'binary'
    const buf = await readFile(abs)
    return buf.subarray(0, 8000).includes(0) ? 'binary' : buf.toString('utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

async function walk(abs: string, out: string[], limit: number): Promise<boolean> {
  let entries
  try {
    entries = await readdir(abs, { withFileTypes: true })
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code
    if (code === 'ENOTDIR') {
      out.push(abs)
      return true
    }
    if (code === 'ENOENT') return true
    throw err
  }
  for (const e of entries) {
    if (SKIP_DIRS.has(e.name)) continue
    const child = path.join(abs, e.name)
    if (e.isDirectory()) {
      if (!(await walk(child, out, limit))) return false
    } else if (e.isFile()) {
      if (out.length >= limit) return false
      out.push(child)
    }
  }
  return true
}

function gitChangedFiles(root: string): Promise<string[]> {
  return new Promise((resolve) => {
    execFile(gitExePath(), ['status', '--porcelain=v1', '-z', '--untracked-files=all'], { cwd: root, timeout: 10_000, windowsHide: true, maxBuffer: 10_000_000 }, (err, stdout) => {
      if (err) {
        resolve([]) // git yok veya depo değil: yalnızca yol argümanları saklanır
        return
      }
      const parts = stdout.split('\0')
      const files: string[] = []
      for (let i = 0; i < parts.length; i++) {
        const entry = parts[i] ?? ''
        if (entry.length < 4) continue
        const status = entry.slice(0, 2)
        files.push(entry.slice(3))
        if (status.startsWith('R') || status.startsWith('C')) i++ // yeniden adlandırmada kaynak yol ayrı alandadır
      }
      resolve(files)
    })
  })
}

function confined(root: string, target: string): string | null {
  try {
    const abs = confineToWorkspace(root, target)
    return isSensitivePath(abs) ? null : abs
  } catch {
    return null // çalışma dizini dışı: komut zaten izin sisteminden geçti, burada saklanmaz
  }
}

export async function takeSnapshot(root: string, plan: RiskyPlan): Promise<Snapshot> {
  const snap: Snapshot = { root, files: new Map(), watch: [], partial: false, skippedBinary: 0 }
  const candidates: string[] = []
  for (const p of plan.paths) {
    const abs = confined(root, p)
    if (!abs) continue
    snap.watch.push(abs)
    if (!(await walk(abs, candidates, SNAPSHOT_MAX_FILES))) snap.partial = true
  }
  if (plan.git) {
    for (const rel of await gitChangedFiles(root)) {
      const abs = confined(root, rel)
      if (abs) candidates.push(abs)
    }
  }
  let chars = 0
  for (const abs of new Set(candidates)) {
    if (snap.files.size >= SNAPSHOT_MAX_FILES || chars > SNAPSHOT_MAX_CHARS) {
      snap.partial = true
      break
    }
    const rel = toPosix(path.relative(root, abs))
    if (isSensitivePath(rel)) continue
    const content = await readText(abs)
    if (content === 'binary') {
      snap.skippedBinary++
      continue
    }
    snap.files.set(rel, content)
    chars += content?.length ?? 0
  }
  return snap
}

/**
 * Komuttan sonra: değişen/silinen dosyalar ve izlenen yollarda yeni oluşanlar bildirilir.
 * onFileChange'in "after" alanı metindir; silinen dosya boş içerikle bildirilir (geri sarma "before"u kullanır).
 */
export async function reportSnapshotChanges(
  snap: Snapshot,
  onFileChange: (relPath: string, before: string | null, after: string) => void,
): Promise<number> {
  let changed = 0
  const seen = new Set(snap.files.keys())
  for (const [rel, before] of snap.files) {
    const after = await readText(path.join(snap.root, rel))
    if (after === 'binary' || after === before) continue
    onFileChange(rel, before, after ?? '')
    changed++
  }
  const created: string[] = []
  for (const abs of snap.watch) await walk(abs, created, SNAPSHOT_MAX_FILES)
  for (const abs of created) {
    const rel = toPosix(path.relative(snap.root, abs))
    if (seen.has(rel) || isSensitivePath(rel) || !confined(snap.root, rel)) continue
    const after = await readText(abs)
    if (typeof after !== 'string') continue
    onFileChange(rel, null, after)
    changed++
  }
  return changed
}
