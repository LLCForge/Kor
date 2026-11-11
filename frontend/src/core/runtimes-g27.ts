// [g27] Kurulu geliştirme araçlarının sürüm satırını ve winget çıktısındaki ilerlemeyi okur.
// Kurulum kimlikleri sabittir; kullanıcı metni kabuğa gitmez.

export interface RuntimeSpec {
  id: string
  name: string
  /** Sırayla denenir. İlki sürüm döndürürse kuruludur. */
  probes: ReadonlyArray<readonly [string, readonly string[]]>
  version: RegExp
  wingetId: string
  /** winget yoksa açılan resmi kurulum sayfası. Sessiz indirme yok. */
  page: string
}

export const RUNTIMES: readonly RuntimeSpec[] = [
  { id: 'node', name: 'Node.js', probes: [['node', ['--version']]], version: /v?(\d+\.\d+\.\d+)/, wingetId: 'OpenJS.NodeJS.LTS', page: 'https://nodejs.org/en/download' },
  { id: 'python', name: 'Python', probes: [['py', ['-3', '--version']], ['python', ['--version']], ['python3', ['--version']]], version: /Python (\d+\.\d+\.\d+)/, wingetId: 'Python.Python.3.12', page: 'https://www.python.org/downloads/' },
  { id: 'rust', name: 'Rust', probes: [['rustc', ['--version']]], version: /rustc (\d+\.\d+\.\d+)/, wingetId: 'Rustlang.Rustup', page: 'https://rustup.rs/' },
  { id: 'go', name: 'Go', probes: [['go', ['version']]], version: /go(\d+\.\d+(?:\.\d+)?)/, wingetId: 'GoLang.Go', page: 'https://go.dev/dl/' },
  { id: 'java', name: 'Java', probes: [['java', ['-version']]], version: /version "([^"]+)"/, wingetId: 'Microsoft.OpenJDK.21', page: 'https://learn.microsoft.com/java/openjdk/download' },
  { id: 'dotnet', name: '.NET', probes: [['dotnet', ['--version']]], version: /(\d+\.\d+\.\d+)/, wingetId: 'Microsoft.DotNet.SDK.8', page: 'https://dotnet.microsoft.com/download' },
  { id: 'php', name: 'PHP', probes: [['php', ['--version']]], version: /PHP (\d+\.\d+\.\d+)/, wingetId: 'PHP.PHP.8.3', page: 'https://windows.php.net/download/' },
  { id: 'ruby', name: 'Ruby', probes: [['ruby', ['--version']]], version: /ruby (\d+\.\d+\.\d+)/, wingetId: 'RubyInstallerTeam.Ruby.3.3', page: 'https://rubyinstaller.org/' },
  { id: 'bun', name: 'Bun', probes: [['bun', ['--version']]], version: /(\d+\.\d+\.\d+)/, wingetId: 'Oven-sh.Bun', page: 'https://bun.sh/' },
  { id: 'deno', name: 'Deno', probes: [['deno', ['--version']]], version: /deno (\d+\.\d+\.\d+)/, wingetId: 'DenoLand.Deno', page: 'https://docs.deno.com/runtime/getting_started/installation/' },
  { id: 'git', name: 'Git', probes: [['git', ['--version']]], version: /git version (\d+\.\d+\.\d+)/, wingetId: 'Git.Git', page: 'https://git-scm.com/download/win' },
  { id: 'docker', name: 'Docker', probes: [['docker', ['--version']]], version: /Docker version (\d+\.\d+\.\d+)/, wingetId: 'Docker.DockerDesktop', page: 'https://docs.docker.com/desktop/setup/install/windows-install/' },
]

export function runtimeById(id: string): RuntimeSpec | undefined {
  return RUNTIMES.find((item) => item.id === id)
}

// İlk açılışta bulunması beklenen temel araçlar; eksikse yükleme ekranı otomatik kurar.
export const REQUIRED_RUNTIME_IDS = ['node', 'python', 'php', 'git', 'rust'] as const

/** Gerekli araçlardan kurulu OLMAYANLARIN id'leri (REQUIRED sırasında). */
export function missingRequired(rows: ReadonlyArray<{ id: string; installed: boolean }>): string[] {
  return REQUIRED_RUNTIME_IDS.filter((id) => !rows.some((row) => row.id === id && row.installed))
}

export function readVersion(spec: RuntimeSpec, text: string): string | null {
  const match = spec.version.exec(text)
  return match?.[1] ?? null
}

export type InstallPhase = 'indiriliyor' | 'kuruluyor' | 'dogrulaniyor' | 'bitti' | 'hata'

export interface InstallTick {
  phase: InstallPhase
  /** null: çıktıda yüzde yok, çubuk belirsiz kalır. 100 yalnız süreç başarıyla bitince. */
  percent: number | null
  line: string
}

const UNITS: Record<string, number> = { b: 1, kb: 1024, mb: 1024 ** 2, gb: 1024 ** 3 }

function bytes(amount: string, unit: string): number {
  return Number(amount) * (UNITS[unit.toLowerCase()] ?? 1)
}

/** winget satırından aşama ve varsa gerçek yüzde. Yüzde uydurmaz. */
export function tickFromLine(line: string): InstallTick {
  const text = line.trim()
  const lower = text.toLowerCase()
  let phase: InstallPhase = 'kuruluyor'
  if (/download|indir/.test(lower)) phase = 'indiriliyor'
  else if (/verif|doğrula|hash/.test(lower)) phase = 'dogrulaniyor'
  else if (/install|kurul/.test(lower)) phase = 'kuruluyor'

  const pct = /(\d{1,3}(?:\.\d+)?)\s*%/.exec(text)
  if (pct?.[1]) {
    const n = Math.min(99, Math.max(0, Math.round(Number(pct[1]))))
    return { phase, percent: n, line: text.slice(0, 240) }
  }
  const ratio = /(\d+(?:\.\d+)?)\s*(b|kb|mb|gb)\s*\/\s*(\d+(?:\.\d+)?)\s*(b|kb|mb|gb)/i.exec(text)
  if (ratio?.[1] && ratio[2] && ratio[3] && ratio[4]) {
    const total = bytes(ratio[3], ratio[4])
    const done = bytes(ratio[1], ratio[2])
    if (total > 0) return { phase, percent: Math.min(99, Math.round((done / total) * 100)), line: text.slice(0, 240) }
  }
  return { phase, percent: null, line: text.slice(0, 240) }
}
