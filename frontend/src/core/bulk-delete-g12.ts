// [g12] Veri kaybı önleme (madde 104): çok sayıda dosya silecek kabuk komutlarında, etkilenen dosya listesiyle ikinci ve açık kullanıcı onayı.
// Karar (kullanıcı tercihi): "izinleri atla" (bypass) modunda bu onay da ATLANIR — bypass "hiçbir şey sorulmaz" demektir. Diğer TÜM
// modlarda onay kalır: toplu silme geri alınamaz (kontrol noktaları kabuk silmelerini izlemez) ve komut güvenilmez model çıktısıdır
// (istem enjeksiyonu). deny kuralları bypass'ta da geçerlidir.
// shortcut: komut satırı sözcüklenerek okunur; python/node gibi programların içinden yapılan silmeler yakalanmaz (sandbox bunu sınırlar).
import { glob, lstat, opendir } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { effectiveCwd, shellStateFor } from './shell-session.ts'

/** Bu sayıda veya daha fazla dosya silinecekse onay istenir. */
export const BULK_DELETE_THRESHOLD = 10
/** Tarama sınırı: kök dizin gibi dev hedeflerde ana süreç uzun süre meşgul olmasın. */
const SCAN_CAP = 2000
const LIST_LIMIT = 50

export interface BulkDeleteRequest {
  command: string
  /** İlk dosyalar (en fazla 50), çalışma dizinine göre */
  files: string[]
  /** Bulunan dosya sayısı; capped ise gerçek sayı daha fazla olabilir */
  total: number
  capped: boolean
  /** Hedefi belirlenemeyen parçalar (değişken, komut ikamesi, find -delete, git clean...) */
  unresolved: string[]
}

export type BulkDeleteConfirm = (request: BulkDeleteRequest, signal: AbortSignal) => Promise<boolean>

const SEPARATORS = new Set([';', '&', '|', '\n', '\r', '(', ')'])
const PREFIXES = new Set(['sudo', 'doas', 'command', 'builtin', 'nohup', 'time', 'exec', 'env'])
const SHELLS = new Set(['sh', 'bash', 'zsh', 'dash', 'cmd', 'powershell', 'pwsh'])
const DELETERS = new Set(['rm', 'del', 'erase', 'rd', 'rmdir', 'ri', 'remove-item', 'unlink', 'shred'])
const CMD_DELETERS = new Set(['del', 'erase', 'rd', 'rmdir'])
const PS_TARGET_PARAMS = new Set(['-path', '-literalpath', '-lp', '-pspath'])
const PS_FILTER_PARAMS = new Set(['-include', '-exclude', '-filter'])
const PS_VALUE_PARAMS = new Set(['-erroraction', '-ea', '-warningaction', '-wa', '-stream', '-credential'])
const CD_COMMANDS = new Set(['cd', 'chdir', 'pushd', 'set-location', 'sl', 'push-location'])

/** Basit sözcükleme: ayraçlarda komut biter, tırnaklar kaldırılır; ters bölü kaçış sayılmaz (Windows yolları). */
export function splitCommands(command: string): string[][] {
  const commands: string[][] = []
  let words: string[] = []
  let word: string | null = null
  let quote: string | null = null
  const endWord = () => {
    if (word !== null) words.push(word)
    word = null
  }
  for (const ch of command) {
    if (quote) {
      if (ch === quote) quote = null
      else word = (word ?? '') + ch
    } else if (ch === '"' || ch === "'") {
      quote = ch
      word ??= ''
    } else if (SEPARATORS.has(ch)) {
      endWord()
      if (words.length > 0) commands.push(words)
      words = []
    } else if (/\s/.test(ch)) {
      endWord()
    } else {
      word = (word ?? '') + ch
    }
  }
  endWord()
  if (words.length > 0) commands.push(words)
  return commands
}

const commandName = (word: string) => word.replace(/^.*[\\/]/, '').replace(/\.(exe|cmd|bat|com)$/i, '').toLowerCase()

interface DeletePlan {
  targets: string[]
  unresolved: string[]
  /** Komut dizin değiştiriyor: sonraki göreli hedefler çalışma dizinine göre çözülemez */
  cwdChanged: boolean
}

function addTarget(raw: string, plan: DeletePlan): void {
  for (const part of raw.split(',').map((p) => p.trim()).filter(Boolean)) {
    // Değişken, komut ikamesi, ~kullanıcı veya önceki bir cd: kabuk açmadan hedef bilinemez.
    if (/[$`%]/.test(part) || /^~[^\\/]/.test(part) || (plan.cwdChanged && !path.isAbsolute(part))) plan.unresolved.push(part)
    else plan.targets.push(part.startsWith('~') ? os.homedir() + part.slice(1) : part)
  }
}

function planCommand(words: readonly string[], plan: DeletePlan, depth: number): void {
  let i = 0
  while (i < words.length && (PREFIXES.has(commandName(words[i] ?? '')) || /^[A-Za-z_]\w*=/.test(words[i] ?? ''))) i++
  const name = commandName(words[i] ?? '')
  const args = words.slice(i + 1)

  if (SHELLS.has(name)) {
    // PowerShell -EncodedCommand (-e, -ec, -enc...): içerik base64, okunamaz.
    const encoded = (name === 'powershell' || name === 'pwsh') && args.some((a) => /^-ec$/i.test(a) || (/^-e\w*$/i.test(a) && 'encodedcommand'.startsWith(a.slice(1).toLowerCase())))
    if (depth >= 3 || encoded) {
      plan.unresolved.push(words.join(' '))
      return
    }
    // İç komut: bayrak değerleri ayırt edilemediğinden her bayrak olmayan sözcükten başlayan kalan kısım ayrıca okunur.
    args.forEach((a, j) => {
      if (!/^[-/]/.test(a)) collect(args.slice(j).join(' '), plan, depth + 1)
    })
    return
  }
  if (name === 'xargs' && args.some((a) => DELETERS.has(commandName(a)))) plan.unresolved.push(words.join(' '))
  if (name === 'find' && args.some((a, j) => a === '-delete' || ((a === '-exec' || a === '-execdir') && DELETERS.has(commandName(args[j + 1] ?? ''))))) {
    plan.unresolved.push(words.join(' '))
  }
  if (CD_COMMANDS.has(name)) plan.cwdChanged = true
  if (name === 'git' && args[0] === 'clean' && args.some((a) => a === '--force' || /^-[a-z]*f/i.test(a))) plan.unresolved.push(words.join(' '))
  if (!DELETERS.has(name)) return

  const before = plan.targets.length + plan.unresolved.length
  // cmd "del /s *.log" alt dizinlerdeki eşleşmeleri de siler.
  const recursivePattern = (name === 'del' || name === 'erase') && args.some((a) => /^\/s$/i.test(a))
  // Ters eğik çizgili (cmd) hedefte dizin/ad ayrımı da / üzerinden yapılır; yoksa posix'te "build\*.js" tek ad sayılır.
  const add = (raw: string) => {
    if (!(recursivePattern && /[*?]/.test(raw))) return addTarget(raw, plan)
    const norm = raw.includes('\\') ? raw.replaceAll('\\', '/') : raw
    addTarget(path.posix.join(path.posix.dirname(norm), '**', path.posix.basename(norm)), plan)
  }
  for (let k = 0; k < args.length; k++) {
    const arg = args[k] ?? ''
    const lower = arg.toLowerCase()
    if (PS_TARGET_PARAMS.has(lower)) add(args[++k] ?? '')
    else if (PS_FILTER_PARAMS.has(lower)) plan.unresolved.push(`${arg} ${args[++k] ?? ''}`)
    else if (PS_VALUE_PARAMS.has(lower)) k++
    else if (arg.startsWith('-')) continue
    else if (CMD_DELETERS.has(name) && /^\/[a-z](:\S*)?$/i.test(arg)) continue
    else add(arg)
  }
  // Hedefsiz silme komutu (ör. "Remove-Item (Get-ChildItem)"): neyin silineceği bilinemez.
  if (plan.targets.length + plan.unresolved.length === before) plan.unresolved.push(words.join(' '))
}

function collect(command: string, plan: DeletePlan, depth: number): void {
  for (const words of splitCommands(command)) planCommand(words, plan, depth)
}

async function expand(target: string, cwd: string): Promise<string[]> {
  // Sondaki ayraç korunur: "rm -rf bag/" sembolik bağlantının hedefindeki dosyaları siler; lstat da ayraçla hedefi izler.
  if (!/[*?[]/.test(target)) return [path.resolve(cwd, target) + (/[\\/]$/.test(target) ? path.sep : '')]
  // cmd'de *.* uzantısız dosyaları da kapsar; glob'da kapsamaz.
  // Ters eğik çizgili (cmd tarzı) hedefler her platformda / ile glob'lanır; cmd'de *.* uzantısız dosyaları da kapsar.
  const pattern = process.platform === 'win32' || target.includes('\\') ? target.replaceAll('\\', '/').replace(/\*\.\*$/, '*') : target
  const matches: string[] = []
  for await (const match of glob(pattern, { cwd })) {
    matches.push(path.resolve(cwd, match))
    if (matches.length >= SCAN_CAP) break
  }
  return matches
}

/** Silinecek dosyaları sayar (dizinler özyinelemeli; bağlantılar izlenmez, rm -r gibi). Okunamayan dizinler belirsiz sayılır. */
async function countFiles(paths: readonly string[], cwd: string, unresolved: string[]): Promise<Pick<BulkDeleteRequest, 'files' | 'total' | 'capped'>> {
  const display = (p: string) => {
    const rel = path.relative(cwd, p)
    return (rel && !rel.startsWith('..') && !path.isAbsolute(rel) ? rel : p).split(path.sep).join('/')
  }
  const files: string[] = []
  const seen = new Set<string>()
  const stack = [...paths]
  let total = 0
  let capped = false
  while (stack.length > 0) {
    if (total >= SCAN_CAP) {
      capped = true
      break
    }
    const current = stack.pop() ?? ''
    if (seen.has(current)) continue
    seen.add(current)
    const info = await lstat(current).catch(() => null) // Olmayan hedef: silinecek bir şey yok.
    if (!info) continue
    if (!info.isDirectory()) {
      total++
      if (files.length < LIST_LIMIT) files.push(display(current))
      continue
    }
    try {
      for await (const entry of await opendir(current)) {
        stack.push(path.join(current, entry.name))
        if (stack.length > SCAN_CAP) break
      }
    } catch {
      unresolved.push(display(current))
    }
  }
  return { files, total, capped }
}

/** Onay gerekmiyorsa null: silme yok ya da az sayıda ve hepsi belirlenebilir dosya. */
export async function analyzeBulkDelete(command: string, cwd: string): Promise<BulkDeleteRequest | null> {
  const plan: DeletePlan = { targets: [], unresolved: [], cwdChanged: false }
  collect(command, plan, 0)
  if (plan.targets.length === 0 && plan.unresolved.length === 0) return null
  const expanded: string[] = []
  for (const target of plan.targets) {
    try {
      expanded.push(...(await expand(target, cwd)))
    } catch {
      plan.unresolved.push(target)
    }
  }
  const counted = await countFiles(expanded, cwd, plan.unresolved)
  if (plan.unresolved.length === 0 && counted.total < BULK_DELETE_THRESHOLD) return null
  return { command, ...counted, unresolved: plan.unresolved }
}

/** CLI ve arayüzsüz onay kanalı için düz metin. */
export function describeBulkDelete(request: BulkDeleteRequest): string {
  const lines = [`Toplu silme: ${request.capped ? 'en az ' : ''}${request.total} dosya silinecek.`, ...request.files.map((f) => `  ${f}`)]
  if (request.total > request.files.length) lines.push(`  ... ve ${request.total - request.files.length}${request.capped ? '+' : ''} dosya daha`)
  if (request.unresolved.length > 0) lines.push(`Hedefi belirlenemeyen kısımlar: ${request.unresolved.join(', ')}`)
  return lines.join('\n')
}

const confirmers = new WeakMap<object, BulkDeleteConfirm>()

/** Kabuk sahibine (oturumun ShellRegistry'si) kullanıcı onay kanalını bağlar; ajan döngüsü her model çağrısından önce yeniler. */
export function setBulkDeleteConfirmer(owner: object, confirm: BulkDeleteConfirm): void {
  confirmers.set(owner, confirm)
}

/** bash aracı komutu başlatmadan önce çağırır; onay alınamazsa veya reddedilirse hata fırlatır (komut çalışmaz). */
export async function guardBulkDelete(command: string, ctx: { root: string; signal: AbortSignal; shells?: object | undefined; permissionMode?: import('./permissions.ts').PermissionMode }): Promise<void> {
  if (ctx.permissionMode === 'bypass') return // izinleri atla: kullanıcı bilinçle tüm onayları kapattı
  const owner = ctx.shells ?? ctx
  const request = await analyzeBulkDelete(command, effectiveCwd(shellStateFor(owner), ctx.root))
  if (!request) return
  const confirm = confirmers.get(owner)
  if (!confirm) throw new Error('Toplu silme için kullanıcı onayı alınamadı; komut çalıştırılmadı.')
  if (!(await confirm(request, ctx.signal))) {
    throw new Error('Kullanıcı toplu silmeyi reddetti. Aynı silmeyi başka bir komut, araç veya yazımla yeniden DENEME; kullanıcıya sor.')
  }
}
