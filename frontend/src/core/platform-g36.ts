// [g36] Ajan hangi işletim sisteminde, hangi kabukta çalıştığını ve hangi komutların gerçekten kurulu olduğunu bilsin;
// yoksa olmayan komutları (python3, grep, ls) deneyip hata üretiyor.
import { existsSync, readFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { ShellSpec } from './shell-session.ts'

/** Okunur işletim sistemi adı ve sürümü: "Windows 11 Pro (build 10.0.26200)", "macOS (Darwin 24.1.0)", "Ubuntu 24.04 LTS". */
export function osLabel(platform: NodeJS.Platform = process.platform): string {
  if (platform === 'win32') return `${os.version()} (build ${os.release()})`
  if (platform === 'darwin') return `macOS (Darwin ${os.release()})`
  if (platform === 'linux') {
    try {
      const pretty = /^PRETTY_NAME="?([^"\n]+)"?/m.exec(readFileSync('/etc/os-release', 'utf8'))?.[1]
      if (pretty) return `${pretty} (Linux ${os.release()})`
    } catch {
      /* os-release yoksa çekirdek sürümü yeter */
    }
    return `Linux ${os.release()}`
  }
  return `${platform} ${os.release()}`
}

/** Ajanın sık çağırdığı komutlar; yalnızca PATH'te bulunanlar isteme yazılır. */
const PROBED_COMMANDS = [
  'python', 'python3', 'py', 'pip', 'node', 'npm', 'npx', 'pnpm', 'yarn', 'bun', 'deno',
  'git', 'php', 'composer', 'java', 'dotnet', 'go', 'cargo', 'rustc', 'gcc', 'clang', 'cl', 'make', 'cmake',
  'docker', 'curl', 'wget', 'grep', 'sed', 'jq', 'winget', 'choco', 'scoop', 'brew', 'apt',
]

export interface FoundCommand {
  name: string
  path: string
  /** Windows'taki Microsoft Store yönlendirme kısayolu (python.exe): gerçek kurulum değildir. */
  storeAlias: boolean
}

/** Komutları verilen ortamın PATH'inde arar (Windows'ta PATHEXT uzantılarıyla). */
export function findCommands(env: NodeJS.ProcessEnv, platform: NodeJS.Platform = process.platform, names: readonly string[] = PROBED_COMMANDS): FoundCommand[] {
  const pathKey = Object.keys(env).find((k) => k.toUpperCase() === 'PATH') ?? 'PATH'
  const dirs = (env[pathKey] ?? '').split(platform === 'win32' ? ';' : ':').filter(Boolean)
  const exts = platform === 'win32' ? (env.PATHEXT ?? '.COM;.EXE;.BAT;.CMD').split(';').filter(Boolean).map((e) => e.toLowerCase()) : ['']
  // Birleştirme ana makinenin yol ayracıyla yapılır: Windows'u Linux'ta taklit eden testlerde (geçici /tmp dizinleri) de dosya bulunsun.
  const join = path.join
  const found: FoundCommand[] = []
  for (const name of names) {
    let hit: string | null = null
    let alias: string | null = null
    for (const dir of dirs) {
      for (const ext of exts) {
        const candidate = join(dir, name + ext)
        if (!existsSync(candidate)) continue
        if (platform === 'win32' && /[\\/]WindowsApps[\\/]?$/i.test(dir) && /^python3?$/i.test(name)) alias ??= candidate
        else hit = candidate
        break
      }
      if (hit) break
    }
    if (hit) found.push({ name, path: hit, storeAlias: false })
    else if (alias) found.push({ name, path: alias, storeAlias: true })
  }
  return found
}

/** Kabuğa özgü sözdizimi kuralları; araç adı "bash" olsa da komutlar seçili kabukta çalışır. */
export function shellRules(shell: ShellSpec): string {
  switch (shell.kind) {
    case 'cmd':
      return [
        'The bash tool runs commands in cmd.exe on Windows, NOT in bash. Write cmd syntax only:',
        '- cd /d "C:\\path with spaces"; variables are %VAR% (the scratch dir is %KOR_SCRATCH%); chain with && (stop on error) or & (always continue).',
        '- dir, type, copy, move, del, rmdir /s /q, mkdir (creates parent folders), where <program> to locate a program, findstr instead of grep.',
        '- cmd has no built-in ls, cat, rm, touch, export or $VAR; grep/sed work only if they appear in the installed list below. For anything beyond simple commands use the powershell tool, or create and read files with the file tools.',
        '- Do not use "start /b": it detaches the program and its output is lost. Use run_in_background: true instead.',
      ].join('\n')
    case 'powershell':
    case 'pwsh':
      return [
        `The bash tool runs commands in ${shell.name}, NOT in bash. Write PowerShell syntax only:`,
        '- Set-Location "C:\\path"; variables are $env:VAR (the scratch dir is $env:KOR_SCRATCH); Get-ChildItem, Get-Content, Copy-Item, Remove-Item -Recurse -Force, Select-String instead of grep.',
        shell.kind === 'powershell' ? '- Windows PowerShell 5.1 has no && or ||: separate commands with ; and check $LASTEXITCODE.' : '- && and || work in PowerShell 7.',
      ].join('\n')
    case 'git-bash':
      return 'The bash tool runs commands in Git Bash on Windows: POSIX commands (grep, sed, ls, rm) work; use forward-slash paths (/c/Users/...); the scratch dir is $KOR_SCRATCH. Windows programs still need Windows-style arguments.'
    case 'sh':
      return process.platform === 'darwin'
        ? 'The bash tool runs commands in /bin/sh on macOS: BSD tools, so sed -i needs an empty suffix (sed -i \'\' ...) and GNU-only flags may not exist. The scratch dir is $KOR_SCRATCH.'
        : 'The bash tool runs commands in /bin/sh on Linux: GNU coreutils are usually available. The scratch dir is $KOR_SCRATCH.'
  }
}

/** Sistem isteminin "# PLATFORM" bölümü: işletim sistemi, kabuk kuralları ve kurulu komutlar. */
export function platformSection(shell: ShellSpec, env: NodeJS.ProcessEnv): string {
  const found = findCommands(env)
  const real = found.filter((c) => !c.storeAlias).map((c) => c.name)
  const aliases = found.filter((c) => c.storeAlias).map((c) => c.name)
  const lines = [
    '# PLATFORM',
    `Operating system: ${osLabel()} on ${process.arch}. Shell for the bash tool: ${shell.name}.`,
    shellRules(shell),
    `Installed commands found on PATH: ${real.length ? real.join(', ') : '(none of the common ones)'}.`,
  ]
  if (aliases.length) lines.push(`${aliases.join(', ')} on PATH is only the Microsoft Store redirect stub, not a real Python install${real.includes('py') ? '; use py instead' : ''}.`)
  lines.push(
    'Use only commands from this list or ones you have just installed. Do not guess between python/python3/py or bash/cmd syntax: pick what this section says exists. If a needed program is missing, install it (winget, choco, scoop, brew or apt, whichever is listed) or tell the user exactly what to install.',
  )
  return lines.join('\n')
}
