// Uyumluluk panelinin saf yardımcıları (node --test ile sınanır).
import type { CompatApplies, CompatSource } from '../../../ipc-compat.ts'

const ECO_NAME: Record<string, string> = {
  kor: 'Kor',
  claude: 'Claude Code',
  codex: 'Codex',
  cursor: 'Cursor',
  windsurf: 'Windsurf',
  cline: 'Cline',
  roo: 'Roo Code',
  kilo: 'Kilo Code',
  copilot: 'GitHub Copilot',
  gemini: 'Gemini CLI',
  aider: 'Aider',
  opencode: 'OpenCode',
  continue: 'Continue',
  kiro: 'Kiro',
  devin: 'Devin',
  other: 'Diğer',
}

export function ecoName(eco: string): string {
  return ECO_NAME[eco] ?? eco
}

export interface ToolSummary {
  eco: string
  /** Farklı kaynak dosya sayısı */
  files: number
  /** Talimat dosyaları (kurallar) */
  rules: number
  /** İzin kuralı, kanca, komut, ajan, skill ve MCP tanımları */
  other: number
}

/** Bulunan izler araç başına: `rules` talimat kaynakları, `other` geri kalan tanımlar. Kor en sonda, diğerleri ada göre. */
export function toolSummary(instructions: readonly { source: CompatSource }[], others: readonly { source: CompatSource }[]): ToolSummary[] {
  const map = new Map<string, { files: Set<string>; rules: number; other: number }>()
  const at = (s: CompatSource) => {
    let e = map.get(s.eco)
    if (!e) map.set(s.eco, (e = { files: new Set(), rules: 0, other: 0 }))
    e.files.add(s.path)
    return e
  }
  for (const i of instructions) at(i.source).rules++
  for (const o of others) at(o.source).other++
  return [...map.entries()]
    .map(([eco, e]) => ({ eco, files: e.files.size, rules: e.rules, other: e.other }))
    .sort((a, b) => (a.eco === 'kor' ? 1 : b.eco === 'kor' ? -1 : ecoName(a.eco).localeCompare(ecoName(b.eco))))
}

/** Kuralın ne zaman uygulandığı, kısa Türkçe. Bilinmeyen biçim `—`. */
export function appliesText(a: unknown): string {
  if (a === null || typeof a !== 'object' || !('kind' in a)) return '—'
  const x = a as CompatApplies
  switch (x.kind) {
    case 'always':
      return 'her zaman'
    case 'globs':
      return `dosyalar: ${x.globs.join(', ')}`
    case 'on_request':
      return 'gerektiğinde'
    case 'manual':
      return 'yalnızca adıyla'
    default:
      return '—'
  }
}

/** Kaynak yolunun son iki parçası (`.cursor/rules/x.mdc`); tam yol başlıkta kalır. */
export function shortPath(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/').filter(Boolean)
  return parts.slice(-3).join('/')
}
