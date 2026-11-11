// [g3] Kod bloğu dilleri. Saf modül: arayüz "Çalıştır" düğmesini göstermek için de kullanır.

const POWERSHELL = new Set(['powershell', 'ps1', 'pwsh', 'ps'])
const SHELLS = new Set(['bash', 'sh', 'shell', 'zsh', 'console', 'cmd', 'bat', 'batch', ...POWERSHELL])

export function isPowerShellLanguage(language: string): boolean {
  return POWERSHELL.has(language.toLowerCase())
}

export function isShellLanguage(language: string): boolean {
  return SHELLS.has(language.toLowerCase())
}

const EXTENSIONS: Record<string, string> = {
  bash: 'sh', sh: 'sh', shell: 'sh', zsh: 'sh', console: 'sh', cmd: 'cmd', bat: 'bat', batch: 'bat',
  powershell: 'ps1', ps1: 'ps1', pwsh: 'ps1', ps: 'ps1',
  typescript: 'ts', ts: 'ts', tsx: 'tsx', javascript: 'js', js: 'js', jsx: 'jsx', json: 'json',
  python: 'py', py: 'py', rust: 'rs', rs: 'rs', go: 'go', java: 'java', kotlin: 'kt', csharp: 'cs', cs: 'cs',
  cpp: 'cpp', 'c++': 'cpp', c: 'c', php: 'php', ruby: 'rb', rb: 'rb', swift: 'swift', sql: 'sql',
  html: 'html', css: 'css', scss: 'scss', yaml: 'yml', yml: 'yml', toml: 'toml', xml: 'xml', markdown: 'md', md: 'md',
  dockerfile: 'Dockerfile', mermaid: 'mmd', tex: 'tex', latex: 'tex',
}

export function suggestFileName(language: string): string {
  const ext = EXTENSIONS[language.toLowerCase()] ?? 'txt'
  return ext === 'Dockerfile' ? ext : `snippet.${ext}`
}
