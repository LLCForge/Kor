// Kurulabilir eklenti/MCP kataloğu (yerleşik öneri listesi). Kullanıcı tek tıkla kurar; hiçbiri varsayılan kurulu/etkin
// değildir. Git eklentileri <USER_CONFIG_DIR>/plugins altına klonlanır (clone kod çalıştırmaz); rea bir MCP sunucusudur,
// mcp.json'a pasif girdi olarak eklenir ve kullanıcı MCP ayarlarından etkinleştirir.
export interface CatalogGitPlugin {
  kind: 'plugin'
  /** Klon klasörü adı (NAME_RE'ye uymalı). */
  name: string
  repo: string
  description: string
  author: string
}
export interface CatalogMcpServer {
  kind: 'mcp'
  id: string
  label: string
  /** npx ile çalıştırılan paket (rea-agents@sürüm); platforma göre komut işleyicide kurulur. */
  npmPackage: string
  mcpArgs: string[]
  description: string
}
export type CatalogEntry = CatalogGitPlugin | CatalogMcpServer

export const PLUGIN_CATALOG: readonly CatalogEntry[] = [
  { kind: 'mcp', id: 'rea', label: 'rea — tersine mühendislik', npmPackage: 'rea-agents@6.8.0', mcpArgs: ['mcp'], description: 'Binary, uygulama ve çalışma zamanı tersine mühendisliği için MCP sunucusu (Node gerektirir).' },
  { kind: 'plugin', name: 'ecc', repo: 'https://github.com/affaan-m/ecc.git', description: 'Ajan harness optimizasyonu: komut, skill, ajan ve kancalar.', author: 'affaan-m' },
  { kind: 'plugin', name: 'claude-mem', repo: 'https://github.com/thedotmack/claude-mem.git', description: 'Hafıza sıkıştırma: oturumlar arası bağlamı korur.', author: 'Alex Newman' },
  { kind: 'plugin', name: 'ponytail', repo: 'https://github.com/DietrichGebert/ponytail.git', description: 'Tembel kıdemli geliştirici modu: en yalın, en kısa çözüm.', author: 'Dietrich Gebert' },
  { kind: 'plugin', name: 'karpathy-guidelines', repo: 'https://github.com/forrestchang/andrej-karpathy-skills.git', description: 'Yaygın LLM kodlama hatalarını azaltan davranış kuralları.', author: 'forrestchang' },
  { kind: 'plugin', name: 'superpowers', repo: 'https://github.com/obra/superpowers.git', description: 'Çekirdek skill kütüphanesi: TDD, hata ayıklama, işbirliği desenleri.', author: 'Jesse Vincent' },
]

export function catalogGitPlugin(name: string): CatalogGitPlugin | undefined {
  return PLUGIN_CATALOG.find((e): e is CatalogGitPlugin => e.kind === 'plugin' && e.name === name)
}
export function catalogMcpServer(id: string): CatalogMcpServer | undefined {
  return PLUGIN_CATALOG.find((e): e is CatalogMcpServer => e.kind === 'mcp' && e.id === id)
}
