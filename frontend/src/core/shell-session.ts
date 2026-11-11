import { execFileSync, spawn, type SpawnOptions } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { closeSync, existsSync, mkdirSync, openSync, rmSync, statSync, writeFileSync, writeSync } from 'node:fs'
import { readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { loadG5SettingsSync, type ShellPreference } from './settings-g5.ts'
import { scrubEnv } from './secrets.ts'
import { commandPolicyEnabled, policyBlockReason } from './sandbox-policy-g26.ts' // [g26]
import { confineToWorkspace } from './workspace.ts'
import { sandboxInvocation } from './sandbox-g12.ts' // [g12]
import { killProcessTree, whenChildDone } from './kill-tree.ts'
import { scratchDir } from './scratch-g33.ts' // [g33] geçici/deneme dosyaları için KOR_SCRATCH
import { freshSystemPath } from './win-path-g34.ts' // [g34] kurulu araçları yeniden başlatmadan bulmak için güncel PATH

export type ShellKind = ShellPreference | 'sh'

export interface ShellSpec {
  kind: ShellKind
  /** Sistem isteminde ve bash aracı açıklamasında görünen ad */
  name: string
  file: string
}

/** Git Bash: bilinen kurulum yerleri, sonra PATH'teki git.exe'nin yanındaki bin\bash.exe. WSL bash'i (System32) bilerek seçilmez. */
export function findGitBash(env: NodeJS.ProcessEnv = process.env): string | null {
  const roots = [env.ProgramFiles, env['ProgramFiles(x86)'], env.LOCALAPPDATA && path.join(env.LOCALAPPDATA, 'Programs')]
  const candidates = roots.filter((r): r is string => Boolean(r)).map((r) => path.join(r, 'Git', 'bin', 'bash.exe'))
  for (const dir of (env.PATH ?? env.Path ?? '').split(path.delimiter).filter(Boolean)) {
    if (existsSync(path.join(dir, 'git.exe'))) candidates.push(path.join(dir, '..', 'bin', 'bash.exe'))
  }
  return candidates.map((c) => path.resolve(c)).find((c) => existsSync(c)) ?? null
}

export function resolveShell(preference: ShellPreference | null, platform: NodeJS.Platform = process.platform): ShellSpec {
  if (platform !== 'win32') return { kind: 'sh', name: '/bin/sh', file: '/bin/sh' }
  if (preference === 'powershell') return { kind: 'powershell', name: 'Windows PowerShell (powershell.exe)', file: 'powershell.exe' }
  if (preference === 'pwsh') return { kind: 'pwsh', name: 'PowerShell 7 (pwsh.exe)', file: 'pwsh.exe' }
  if (preference === 'git-bash') {
    const bash = findGitBash()
    if (bash) return { kind: 'git-bash', name: 'Git Bash (bash.exe)', file: bash }
    console.warn('Ayarlarda shell: "git-bash" seçili ama Git Bash bulunamadı; cmd.exe kullanılıyor.')
  }
  return { kind: 'cmd', name: 'cmd.exe', file: process.env.ComSpec ?? 'cmd.exe' }
}

let current: ShellSpec = resolveShell(loadG5SettingsSync().shell)

/** Kullanıcı ayarındaki kabuk; modül yüklenirken okunur (değişiklik yeniden başlatınca geçerli olur). */
export function currentShell(): ShellSpec {
  return current
}

/** Testler ve ayar yenileme için. */
export function setShellPreference(preference: ShellPreference | null): ShellSpec {
  current = resolveShell(preference)
  return current
}

interface Invocation {
  file: string
  args: string[]
  options: Pick<SpawnOptions, 'windowsVerbatimArguments'>
}

/** Komutu seçili kabukta çalıştıracak süreç tanımı (durum yakalamasız; arka plan komutları için). */
export function invocationFor(shell: ShellSpec, script: string): Invocation {
  switch (shell.kind) {
    case 'cmd':
      // Node'un shell: true davranışıyla aynı: cmd kendi tırnak kurallarını kullanır.
      return { file: shell.file, args: ['/d', '/s', '/c', `"${script}"`], options: { windowsVerbatimArguments: true } }
    case 'powershell':
    case 'pwsh':
      // EncodedCommand: tırnak/kaçış sorunu yok; yürütme ilkesi yalnızca betik dosyalarına uygulanır.
      return {
        file: shell.file,
        args: ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')],
        options: {},
      }
    case 'git-bash':
    case 'sh':
      return { file: shell.file, args: ['-c', script], options: {} }
  }
}

/** Oturum başına kabuk durumu: çağrılar arasında korunan çalışma dizini ve ortam değişkeni değişiklikleri. */
export interface ShellState {
  cwd: string | null
  /** null: değişken silindi */
  env: Map<string, string | null>
}

const states = new WeakMap<object, ShellState>()

/** Durum, oturumun ShellRegistry nesnesine bağlıdır (alt ajanlar üst oturumunkini paylaşır). */
export function shellStateFor(owner: object): ShellState {
  let state = states.get(owner)
  if (!state) {
    state = { cwd: null, env: new Map() }
    states.set(owner, state)
  }
  return state
}

const sameName = (a: string, b: string) => (process.platform === 'win32' ? a.toUpperCase() === b.toUpperCase() : a === b)

export function buildEnv(state: ShellState, base: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  // Uygulamanın anahtarları ajanın komutlarına geçmez; oturumda açıkça export edilenler aşağıda yine eklenir.
  const env = scrubEnv(base)
  // [g34] Windows: uygulama açıkken kurulan araçları (ör. winget ile php) yeniden başlatmadan bulabilmek için kayıt
  // defterindeki güncel PATH'i, uygulamanın eklediği dizinleri koruyacak şekilde mevcut PATH'in önüne ekle.
  const fresh = freshSystemPath(base)
  if (fresh) {
    let existing = ''
    for (const key of Object.keys(env)) {
      if (!sameName(key, 'Path')) continue
      existing = env[key] ?? existing
      delete env[key]
    }
    env.Path = existing ? `${fresh};${existing}` : fresh
  }
  for (const [name, value] of state.env) {
    // Windows'ta ad büyük/küçük harf duyarsız: "Path" ile "PATH" iki ayrı anahtar olmasın.
    for (const key of Object.keys(env)) if (sameName(key, name)) delete env[key]
    if (value !== null) env[name] = value
  }
  return env
}

/** Kayıtlı dizin silinmiş veya artık dışarıdaysa çalışma köküne dönülür. */
export function effectiveCwd(state: ShellState, root: string): string {
  if (!state.cwd) return root
  try {
    const abs = confineToWorkspace(root, state.cwd)
    if (statSync(abs).isDirectory()) return abs
  } catch {
    // Aşağıda köke dönülür.
  }
  state.cwd = null
  return root
}

const STATE_VAR = '__VA_STATE'
// Kabuğun kendi yönettiği değişkenler kalıcılaştırılmaz.
const IGNORED_ENV = new Set(['_', 'SHLVL', 'PWD', 'OLDPWD', STATE_VAR])

const utf8Strict = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true })
// CP857 0x80-0xFF. Node'un TextDecoder'ı (WHATWG) bu kod sayfasını bilmez.
const CP857 =
  'ÇüéâäàåçêëèïîıÄÅÉæÆôöòûùİÖÜø£ØŞşáíóúñÑĞğ¿®¬½¼¡«»░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐└┴┬├─┼ãÃ╚╔╩╦╠═╬¤ºªÊËÈ�ÍÎÏ┘┌█▄¦Ì▀ÓßÔÒõÕµ�×ÚÛÙìÿ¯´­±�¾¶§÷¸°¨·¹³²■ '

export interface CodePages {
  oem: number
  ansi: number
}

let systemPages: CodePages | null = null

function windowsCodePages(): CodePages {
  if (systemPages) return systemPages
  const read = (name: string): number => {
    try {
      const out = execFileSync('reg', ['query', 'HKLM\\SYSTEM\\CurrentControlSet\\Control\\Nls\\CodePage', '/v', name], { windowsHide: true, timeout: 3000, encoding: 'latin1', stdio: ['ignore', 'pipe', 'ignore'] })
      return Number(/(\d+)\s*$/.exec(out)?.[1] ?? 0)
    } catch {
      return 0
    }
  }
  systemPages = { oem: read('OEMCP'), ansi: read('ACP') }
  return systemPages
}

/** chcp'yi dinlemeyen programlar yerel kod sayfasıyla yazar: Türkçe harfler CP857'de 0x80-0xA7, CP1254'te 0xC7-0xFE aralığındadır. */
function decodeLegacyLine(line: Buffer, pages: CodePages): string {
  const high = line.filter((b) => b >= 0x80)
  if (pages.oem === 857 && high.filter((b) => b < 0xc0).length * 2 >= high.length) {
    return Array.from(line, (b) => (b < 0x80 ? String.fromCharCode(b) : CP857[b - 0x80])).join('')
  }
  try {
    return new TextDecoder(`windows-${pages.ansi}`).decode(line)
  } catch {
    return line.toString('utf8')
  }
}

/** Önce katı UTF-8; olmazsa satır satır yerel kod sayfası (aynı çıktıda ikisi karışabilir). */
export function decodeOutputBytes(bytes: Buffer, pages?: CodePages): string {
  try {
    return utf8Strict.decode(bytes)
  } catch {
    if (!pages && process.platform !== 'win32') return bytes.toString('utf8')
  }
  const resolved = pages ?? windowsCodePages()
  const parts: string[] = []
  let start = 0
  while (start <= bytes.length) {
    const nl = bytes.indexOf(0x0a, start)
    const line = bytes.subarray(start, nl === -1 ? bytes.length : nl)
    try {
      parts.push(utf8Strict.decode(line))
    } catch {
      parts.push(decodeLegacyLine(line, resolved))
    }
    if (nl === -1) break
    start = nl + 1
  }
  return parts.join('\n')
}

/** Parça sonundaki tamamlanmamış UTF-8 dizisinin bayt sayısı. */
function incompleteUtf8Tail(buf: Buffer): number {
  for (let back = 1; back <= Math.min(3, buf.length); back++) {
    const byte = buf[buf.length - back] ?? 0
    if ((byte & 0xc0) === 0x80) continue
    const need = byte >= 0xf0 ? 4 : byte >= 0xe0 ? 3 : byte >= 0xc0 ? 2 : 1
    return need > back ? back : 0
  }
  return 0
}

/** Akış çözücü: çok baytlı karakter iki parçaya bölünse de bozulmaz. Her akışa (stdout/stderr) ayrı örnek. */
export class OutputDecoder {
  #pending: Buffer = Buffer.alloc(0)

  write(chunk: Buffer): string {
    const buf = this.#pending.length > 0 ? Buffer.concat([this.#pending, chunk]) : chunk
    const keep = incompleteUtf8Tail(buf)
    this.#pending = Buffer.from(buf.subarray(buf.length - keep))
    return decodeOutputBytes(buf.subarray(0, buf.length - keep))
  }

  end(): string {
    const rest = this.#pending
    this.#pending = Buffer.alloc(0)
    return rest.length > 0 ? decodeOutputBytes(rest) : ''
  }
}

// Kaçak bir komut (ör. `yes`) diski doldurmasın.
const MAX_SPILL_BYTES = 256 * 1024 * 1024
// Başlık notu ve durum satırı için pay: sonuç çağıranın genel sınırında kalır, ikinci kez kırpılmaz.
const NOTE_RESERVE_CHARS = 400

/**
 * Uzun çıktı: bellekte baş ve kayan bir son tutulur, sınır aşıldığı andan itibaren tamamı iş alanındaki
 * dosyaya akar. Baştan kesip sonu atmak hatanın en sonda olduğu komutlarda teşhisi imkânsız kılıyordu.
 */
export class OutputCapture {
  readonly #root: string
  readonly #max: number
  readonly #headMax: number
  readonly #tailMax: number
  #text = ''
  #head = ''
  #total = 0
  #overflowed = false
  #fd: number | null = null
  #spillRel: string | null = null
  #spilled = 0

  constructor(root: string, maxChars: number) {
    this.#root = root
    this.#max = maxChars
    this.#headMax = Math.floor(maxChars * 0.45)
    this.#tailMax = Math.max(1, maxChars - this.#headMax - NOTE_RESERVE_CHARS)
  }

  add(text: string): void {
    if (!text) return
    this.#total += text.length
    if (!this.#overflowed && this.#total <= this.#max) {
      this.#text += text
      return
    }
    if (!this.#overflowed) {
      this.#overflowed = true
      this.#openSpill()
      this.#spill(this.#text)
      this.#head = this.#text.slice(0, this.#headMax)
      this.#text = this.#text.slice(this.#headMax)
    }
    this.#spill(text)
    this.#text += text
    if (this.#text.length > this.#tailMax * 2) this.#text = this.#text.slice(-this.#tailMax)
  }

  /** Sonuç metni; dosyayı kapatır. Not yalnızca gerçekleşeni söyler. */
  finish(): string {
    if (this.#fd !== null) {
      try {
        closeSync(this.#fd)
      } catch {
        // Yazılanlar zaten diskte.
      }
      this.#fd = null
    }
    if (!this.#overflowed) return this.#text
    const tail = this.#text.slice(-this.#tailMax)
    const file = this.#spillRel ? (this.#spilled >= MAX_SPILL_BYTES ? `; ilk ${MAX_SPILL_BYTES / 1024 / 1024} MB: ${this.#spillRel}` : `; tamamı: ${this.#spillRel}`) : ''
    return `${this.#head}\n... [${this.#total - this.#head.length - tail.length} karakter atlandı${file}; çıktının başı ve sonu korundu] ...\n${tail}`
  }

  #openSpill(): void {
    try {
      const dir = path.join(this.#root, '.kor', 'out')
      mkdirSync(dir, { recursive: true })
      const file = path.join(dir, `cikti-${Date.now()}-${Math.random().toString(36).slice(2, 8)}.txt`)
      this.#fd = openSync(file, 'w')
      this.#spillRel = path.relative(this.#root, file).split(path.sep).join('/')
    } catch {
      this.#fd = null // Dosya yoksa not da dosyadan söz etmez.
    }
  }

  #spill(text: string): void {
    if (this.#fd === null || this.#spilled >= MAX_SPILL_BYTES) return
    try {
      const bytes = Buffer.from(text, 'utf8')
      writeSync(this.#fd, bytes)
      this.#spilled += bytes.length
    } catch {
      // Disk doldu vb.: not artık "tamamı dosyada" diyemez.
      try {
        closeSync(this.#fd)
      } catch {
        // Yapılacak bir şey yok.
      }
      this.#fd = null
      this.#spillRel = null
    }
  }
}

const CMD_COMMENT = /^\s*@?(?:rem\b|::)/i

/**
 * Tek satırlık `cmd /c` anlamı korunur: `for %i`, `curl a%20b`, `echo 100%` çalışır; `%AD%` ve zaten `%%` olanlar olduğu gibi kalır.
 * Toplu dosyada tek başına duran % başka anlama gelirdi (`%20` -> `%2` argümanı -> boş).
 */
function cmdPercents(line: string): string {
  return line.replace(/%%|%(?:[A-Za-z_#$][\w#$.()-]*(?::[^%\r\n]*)?%)?/g, (m) => (m === '%' ? '%%' : m))
}

/** Mantıksal satırlar: satır sonu ( ile açılan blok ve ^ ile devam eden satır bölünmez. */
function cmdSections(command: string): string[] {
  const sections: string[] = []
  let current: string[] = []
  let depth = 0
  for (const line of command.split(/\r?\n/)) {
    if (current.length === 0 && !line.trim()) continue
    current.push(cmdPercents(line))
    if (!CMD_COMMENT.test(line)) depth = Math.max(0, depth + (/\(\s*$/.test(line) ? 1 : 0) - (/^\s*\)/.test(line) ? 1 : 0))
    if (depth > 0 || /\^\s*$/.test(line)) continue
    sections.push(current.join('\r\n'))
    current = []
  }
  if (current.length > 0) sections.push(current.join('\r\n'))
  return sections
}

/**
 * Komut satırı sınırlarını aşan geçici toplu dosya. Her mantıksal satır dosyanın kendisine `call` ile girilen ayrı bir
 * bağlamda çalışır: `call`sız `npm test` (npm.cmd) yalnızca o bağlamı devralır, sonraki satırlar ve durum yakalama yine çalışır.
 * `rem`, tek tırnak veya sondaki `&` de sonraki satırı yutamaz. Kullanıcı etiket yazdıysa gerçek toplu betiktir: tek parça kalır.
 * chcp: windowsHide + devralınmayan stdio ile süreç CREATE_NO_WINDOW alır, yani gizli ve özel konsolu olur; kullanıcının
 * konsolu CLI'da da değişmez (ölçüldü). Dosya UTF-8 okunur, ilk satırlar bu yüzden ASCII'dir.
 */
function cmdBatch(command: string, capture: boolean): string {
  const sections = /^\s*:[^:\s]/m.test(command) ? [command.replace(/\r?\n/g, '\r\n')] : cmdSections(command)
  const state = `%${STATE_VAR}%`
  return [
    '@echo off',
    'if not "%~1"=="" goto %~1',
    'chcp 65001>nul 2>&1',
    ...(capture ? [`cmd /d /u /c set>"${state}.before"`] : []),
    ...sections.map((_, i) => `call "%~f0" __va${i}`),
    // Dökümler iç içe "cmd /u" ile UTF-16 yazılır: Türkçe yollar kod sayfasından bağımsız doğru çözülür.
    ...(capture ? [`>"${state}.ec" echo %ERRORLEVEL%`, `cmd /d /u /c cd>"${state}.cwd"`, `cmd /d /u /c set>"${state}.after"`] : []),
    'exit /b %ERRORLEVEL%',
    ...sections.flatMap((body, i) => [`:__va${i}`, body, 'goto :eof']),
    '',
  ].join('\r\n')
}

/**
 * Yöntem (madde 131): kalıcı süreç yerine her çağrıda yeni kabuk + durum aktarımı. Kalıcı süreçte stdin'i
 * okuyan komutlar protokolü bozar, zaman aşımında tüm durum kaybolur ve cmd/PowerShell'in stdin modu
 * kırılgandır. Burada komut, öncesinde ve sonrasında ortamı dosyaya döken bir sarmalayıcıyla çalışır;
 * yalnızca komutun yaptığı değişiklikler (fark) saklanır, çalışma dizini çalışma alanı içinde kalmalıdır.
 */
export function wrapScript(shell: ShellSpec, command: string): string {
  switch (shell.kind) {
    case 'cmd':
      // Dönen metin geçici .cmd dosyasının içeriğidir (bkz. cmdBatch).
      return cmdBatch(command, true)
    case 'powershell':
    case 'pwsh':
      return [
        '$__va = $env:__VA_STATE',
        '$__vaUtf8 = New-Object System.Text.UTF8Encoding $false',
        // Konsol paylaşılıyorsa (CLI) kod sayfası değişikliği finally'de geri alınır.
        '$__vaPrevOut = [Console]::OutputEncoding',
        '[Console]::OutputEncoding = $__vaUtf8',
        '$OutputEncoding = $__vaUtf8',
        '[IO.File]::WriteAllText("$__va.before", (ConvertTo-Json -Compress ([Environment]::GetEnvironmentVariables())), $__vaUtf8)',
        '$global:LASTEXITCODE = 0',
        '$__vaOk = $true',
        'try {',
        command,
        '$__vaOk = $?',
        '} catch { $__vaOk = $false; Write-Error -ErrorRecord $_ } finally {',
        '[Console]::OutputEncoding = $__vaPrevOut',
        '[IO.File]::WriteAllText("$__va.cwd", (Get-Location).ProviderPath, $__vaUtf8)',
        '[IO.File]::WriteAllText("$__va.after", (ConvertTo-Json -Compress ([Environment]::GetEnvironmentVariables())), $__vaUtf8)',
        '}',
        'if ($global:LASTEXITCODE) { exit $global:LASTEXITCODE }',
        'if (-not $__vaOk) { exit 1 }',
        'exit 0',
      ].join('\n')
    case 'git-bash':
    case 'sh': {
      const pwd = shell.kind === 'git-bash' ? '{ pwd -W 2>/dev/null || pwd; }' : 'pwd'
      return [
        `__va="$${STATE_VAR}"`,
        'env -0 > "$__va.before" 2>/dev/null',
        `trap '__va_ec=$?; ${pwd} > "$__va.cwd"; env -0 > "$__va.after" 2>/dev/null; exit $__va_ec' EXIT`,
        command,
      ].join('\n')
    }
  }
}

type DumpFormat = 'lines' | 'nul' | 'json'

export function parseEnvDump(text: string, format: DumpFormat): Map<string, string> {
  const vars = new Map<string, string>()
  if (format === 'json') {
    let json: unknown
    try {
      json = JSON.parse(text.replace(/^﻿/, ''))
    } catch {
      return vars
    }
    if (json && typeof json === 'object') {
      for (const [k, v] of Object.entries(json)) if (typeof v === 'string') vars.set(k, v)
    }
    return vars
  }
  for (const entry of text.split(format === 'nul' ? '\0' : /\r?\n/)) {
    const eq = entry.indexOf('=', 1)
    if (eq > 0) vars.set(entry.slice(0, eq), entry.slice(eq + 1))
  }
  return vars
}

/** Yalnızca komutun değiştirdiği değişkenler: aynı kabuğun önceki/sonraki dökümleri karşılaştırılır. */
export function diffEnv(before: Map<string, string>, after: Map<string, string>, ignored: ReadonlySet<string>): Map<string, string | null> {
  const changes = new Map<string, string | null>()
  if (before.size === 0 || after.size === 0) return changes // Döküm alınamadı: değişiklik varsayılmaz.
  for (const [name, value] of after) if (!ignored.has(name) && before.get(name) !== value) changes.set(name, value)
  for (const name of before.keys()) if (!ignored.has(name) && !after.has(name)) changes.set(name, null)
  return changes
}

export interface ShellRunResult {
  code: number | null
  signal: NodeJS.Signals | null
  output: string
  timedOut: boolean
  /** Modele gösterilecek durum notları (ör. reddedilen cd) */
  notes: string[]
}

async function readOptional(file: string, encoding: 'utf-8' | 'utf-16le'): Promise<string | null> {
  try {
    return new TextDecoder(encoding).decode(await readFile(file))
  } catch {
    return null // Komut exit/exec ile erken bittiyse döküm olmayabilir: durum değişmez.
  }
}

export interface ShellRunContext {
  root: string
  signal: AbortSignal
  /** Durumun bağlandığı nesne (oturumun ShellRegistry'si) */
  owner: object
}

/** Ön plan komutu: seçili kabukta, oturumun dizini ve ortamıyla çalışır; sonra durum güncellenir. */
export async function runShellCommand(command: string, ctx: ShellRunContext, timeoutMs: number, maxOutputChars: number): Promise<ShellRunResult> {
  const shell = currentShell()
  const state = shellStateFor(ctx.owner)
  const cwd = effectiveCwd(state, ctx.root)
  if (commandPolicyEnabled()) {
    const blocked = policyBlockReason(command, ctx.root)
    if (blocked) throw new Error(`Komut politikası engelledi: ${blocked}`)
  }
  const stateBase = path.join(os.tmpdir(), `venice-shell-${randomUUID()}`)
  // [g12] Sandbox açıksa bwrap ile sarılır; kurulamıyorsa burada hata fırlatılır ve komut çalışmaz.
  const stateFiles = ['.before', '.after', '.cwd', '.ec'].map((ext) => stateBase + ext)
  const script = wrapScript(shell, command)
  const batchFile = `${stateBase}.cmd`
  const inv = sandboxInvocation(invocationFor(shell, shell.kind === 'cmd' ? `"${batchFile}"` : script), { root: ctx.root, cwd, stateFiles })
  const timeout = AbortSignal.timeout(timeoutMs)
  const combined = AbortSignal.any([ctx.signal, timeout])
  const notes: string[] = []

  try {
    if (shell.kind === 'cmd') await writeFile(batchFile, script, { mode: 0o600 })
    const result = await new Promise<Omit<ShellRunResult, 'notes'>>((resolve, reject) => {
      // signal bilerek verilmez: önce kabuk ölürse torunlar öksüz kalır, taskkill /T onlara ulaşamazdı.
      const child = spawn(inv.file, inv.args, {
        ...inv.options,
        cwd,
        env: { ...buildEnv(state), [STATE_VAR]: stateBase, KOR_SCRATCH: scratchDir(ctx.root) },
        stdio: ['ignore', 'pipe', 'pipe'], // set /p, pause zaman aşımına kadar beklemesin
        detached: process.platform !== 'win32', // POSIX'te süreç grubu: killProcessTree torunlara da ulaşsın
        windowsHide: true,
      })
      const capture = new OutputCapture(ctx.root, maxOutputChars)
      const outDecoder = new OutputDecoder()
      const errDecoder = new OutputDecoder()
      child.stdout?.on('data', (chunk: Buffer) => capture.add(outDecoder.write(chunk)))
      child.stderr?.on('data', (chunk: Buffer) => capture.add(errDecoder.write(chunk)))
      child.on('error', reject)
      const armDeadline = whenChildDone(child, (code, signal) => {
        combined.removeEventListener('abort', onAbort)
        capture.add(outDecoder.end() + errDecoder.end())
        resolve({ code, signal, output: capture.finish(), timedOut: timeout.aborted })
      })
      function onAbort(): void {
        killProcessTree(child.pid)
        armDeadline()
      }
      if (combined.aborted) onAbort()
      else combined.addEventListener('abort', onAbort, { once: true })
    })

    const format: DumpFormat = shell.kind === 'cmd' ? 'lines' : shell.kind === 'powershell' || shell.kind === 'pwsh' ? 'json' : 'nul'
    const dumpEncoding = shell.kind === 'cmd' ? 'utf-16le' : 'utf-8'
    const [before, after, cwdText, ecText] = await Promise.all([
      readOptional(`${stateBase}.before`, dumpEncoding),
      readOptional(`${stateBase}.after`, dumpEncoding),
      readOptional(`${stateBase}.cwd`, dumpEncoding),
      readOptional(`${stateBase}.ec`, 'utf-8'),
    ])
    const ignored = new Set(IGNORED_ENV)
    // Git Bash PATH'i POSIX biçimine çevirir; geri yazılırsa Windows komutları bozulur.
    if (shell.kind === 'git-bash') ignored.add('PATH')
    if (before !== null && after !== null) {
      for (const [name, value] of diffEnv(parseEnvDump(before, format), parseEnvDump(after, format), ignored)) state.env.set(name, value)
    }
    const nextCwd = cwdText?.trim()
    if (nextCwd) {
      let accepted: string | null = null
      try {
        const abs = confineToWorkspace(ctx.root, nextCwd)
        if (statSync(abs).isDirectory()) accepted = abs
      } catch {
        accepted = null
      }
      if (accepted) state.cwd = path.relative(ctx.root, accepted) === '' ? null : accepted
      else notes.push(`cd reddedildi: ${nextCwd} çalışma dizini dışında; dizin ${path.relative(ctx.root, cwd) || '.'} olarak kaldı`)
    }
    const code = shell.kind === 'cmd' && ecText !== null && /^-?\d+\s*$/.test(ecText) && !result.timedOut ? Number(ecText.trim()) : result.code
    const now = effectiveCwd(state, ctx.root)
    if (now !== ctx.root) notes.push(`çalışma dizini: ${path.relative(ctx.root, now) || now}`)
    return { ...result, code, notes }
  } finally {
    // Arkada kalan torun bir dosyayı açık tutuyorsa silme hatası komut sonucunu gölgelemesin.
    await Promise.all(['.before', '.after', '.cwd', '.ec', '.cmd'].map((ext) => rm(stateBase + ext, { force: true }).catch(() => undefined)))
  }
}

/** Arka plan komutu: seçili kabuk, oturumun dizini ve ortamı; durum geri yazılmaz. cleanup süreç bitince çağrılır. */
export function backgroundSpawn(command: string, ctx: ShellRunContext): { file: string; args: string[]; options: SpawnOptions; cleanup?: () => void } {
  const state = shellStateFor(ctx.owner)
  const cwd = effectiveCwd(state, ctx.root)
  if (commandPolicyEnabled()) {
    const blocked = policyBlockReason(command, ctx.root)
    if (blocked) throw new Error(`Komut politikası engelledi: ${blocked}`)
  }
  const shell = currentShell()
  let script = command
  let cleanup: (() => void) | undefined
  if (shell.kind === 'cmd') {
    const file = path.join(os.tmpdir(), `venice-shell-${randomUUID()}.cmd`)
    writeFileSync(file, cmdBatch(command, false), { mode: 0o600 })
    script = `"${file}"`
    cleanup = () => rmSync(file, { force: true })
  } else if (shell.kind === 'powershell' || shell.kind === 'pwsh') {
    script = `[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false\n${command}`
  }
  const inv = sandboxInvocation(invocationFor(shell, script), { root: ctx.root, cwd }) // [g12]
  return { file: inv.file, args: inv.args, options: { ...inv.options, cwd, env: { ...buildEnv(state), KOR_SCRATCH: scratchDir(ctx.root) } }, ...(cleanup ? { cleanup } : {}) }
}
