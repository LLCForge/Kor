import { access, readFile } from 'node:fs/promises'
import path from 'node:path'

export const EDITOR_IDS = ['code', 'cursor', 'windsurf', 'zed'] as const
export type EditorId = (typeof EDITOR_IDS)[number]

export interface EditorLaunch {
  command: string
  args: string[]
  /** Alt sürece eklenecek ortam değişkenleri */
  env: Record<string, string>
}

type Env = Readonly<Record<string, string | undefined>>

/** GUI'den başlatılan uygulamada PATH eksik olabilir (özellikle macOS); bilinen kurulum yerlerine de bakılır. */
function wellKnown(id: EditorId, platform: NodeJS.Platform, env: Env): string[] {
  if (platform === 'win32') {
    const local = env.LOCALAPPDATA ?? ''
    const programFiles = env.ProgramFiles ?? 'C:\\Program Files'
    switch (id) {
      case 'code':
        return [path.win32.join(local, 'Programs', 'Microsoft VS Code', 'bin', 'code.cmd'), path.win32.join(programFiles, 'Microsoft VS Code', 'bin', 'code.cmd')]
      case 'cursor':
        return [path.win32.join(local, 'Programs', 'cursor', 'resources', 'app', 'bin', 'cursor.cmd')]
      case 'windsurf':
        return [path.win32.join(local, 'Programs', 'Windsurf', 'bin', 'windsurf.cmd')]
      case 'zed':
        return [path.win32.join(local, 'Programs', 'Zed', 'zed.exe')]
    }
  }
  if (platform === 'darwin') {
    switch (id) {
      case 'code':
        return ['/usr/local/bin/code', '/opt/homebrew/bin/code', '/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code']
      case 'cursor':
        return ['/usr/local/bin/cursor', '/opt/homebrew/bin/cursor', '/Applications/Cursor.app/Contents/Resources/app/bin/cursor']
      case 'windsurf':
        return ['/usr/local/bin/windsurf', '/opt/homebrew/bin/windsurf', '/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf']
      case 'zed':
        return ['/usr/local/bin/zed', '/opt/homebrew/bin/zed', '/Applications/Zed.app/Contents/MacOS/cli']
    }
  }
  switch (id) {
    case 'code':
      return ['/usr/bin/code', '/usr/local/bin/code', '/snap/bin/code']
    case 'cursor':
      return ['/usr/bin/cursor', '/usr/local/bin/cursor']
    case 'windsurf':
      return ['/usr/bin/windsurf', '/usr/local/bin/windsurf']
    case 'zed':
      return ['/usr/bin/zed', '/usr/local/bin/zed', '/snap/bin/zed']
  }
}

export function candidatePaths(id: EditorId, platform: NodeJS.Platform = process.platform, env: Env = process.env): string[] {
  const p = platform === 'win32' ? path.win32 : path.posix
  const names = platform === 'win32' ? [`${id}.cmd`, `${id}.exe`] : [id]
  const dirs = (env.PATH ?? env.Path ?? '').split(platform === 'win32' ? ';' : ':').filter((d) => d.trim() !== '')
  return [...dirs.flatMap((d) => names.map((n) => p.join(d, n))), ...wellKnown(id, platform, env)]
}

export async function findEditor(id: EditorId): Promise<string | null> {
  for (const candidate of candidatePaths(id)) {
    try {
      await access(candidate)
      return candidate
    } catch {
      // Bu konumda yok; sıradaki aday denenir.
    }
  }
  return null
}

/**
 * VS Code / Cursor Windows sarmalayıcısı (code.cmd): `"%~dp0..\Code.exe" "%~dp0..\resources\app\out\cli.js" %*`.
 * .cmd dosyası kabuk olmadan çalıştırılamaz; kabuk da dosya adındaki & gibi karakterlerle komut enjeksiyonuna açıktır.
 * Bu yüzden sarmalayıcının yaptığını doğrudan, argüman dizisiyle yaparız.
 */
export function parseCmdShim(content: string, cmdDir: string): { exe: string; cli: string } | null {
  const m = /"%~dp0([^"]+?\.exe)"\s+"%~dp0([^"]+?\.js)"/i.exec(content)
  if (!m?.[1] || !m[2]) return null
  return { exe: path.win32.resolve(cmdDir, m[1]), cli: path.win32.resolve(cmdDir, m[2]) }
}

/** file mutlak yoldur, bu yüzden "-" ile başlayıp seçenek sanılamaz. */
export async function launchFor(found: string, file: string, line: number | null, platform: NodeJS.Platform = process.platform, opts: { sshHost?: string; zed?: boolean } = {}): Promise<EditorLaunch | null> {
  // Zed: `zed dosya:satır` (-g yok). VS Code ailesi: `-g dosya:satır`; uzak SSH için `--remote ssh-remote+host`.
  const loc = line !== null && line > 0 ? `${file}:${line}` : file
  const remote = opts.sshHost && !opts.zed ? ['--remote', `ssh-remote+${opts.sshHost}`] : []
  const target = opts.zed ? [loc] : [...remote, ...(line !== null && line > 0 ? ['-g', loc] : [file])]
  if (platform === 'win32' && /\.cmd$/i.test(found)) {
    const shim = parseCmdShim(await readFile(found, 'utf8'), path.win32.dirname(found))
    return shim ? { command: shim.exe, args: [shim.cli, ...target], env: { ELECTRON_RUN_AS_NODE: '1' } } : null
  }
  return { command: found, args: target, env: {} }
}

/** Varsayılan uygulamayla açılınca çalıştırılabilecek türler: bunlar açılmaz, klasörde gösterilir. */
const EXECUTABLE_EXT = new Set([
  '.exe', '.bat', '.cmd', '.com', '.ps1', '.psm1', '.vbs', '.vbe', '.js', '.jse', '.mjs', '.cjs', '.wsf', '.wsh', '.msi', '.msp',
  '.lnk', '.scr', '.hta', '.cpl', '.reg', '.jar', '.app', '.sh', '.command', '.desktop', '.appimage', '.run', '.py', '.pyw',
  '.url', '.inf', '.application', '.gadget', '.pif', '.scf', '.dll', '.sys',
])

export function isRiskyToOpen(file: string): boolean {
  return EXECUTABLE_EXT.has(path.extname(file).toLowerCase())
}
