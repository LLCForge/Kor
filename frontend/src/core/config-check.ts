// Başlangıçta yapılandırma doğrulaması: her ayar dosyası kendi zod şemasıyla okunur; hatalı dosya uygulamayı ilk
// kullanımda sessizce bozmak yerine öz-denetimde "dosya: sorun" olarak listelenir. Hiçbir dosya değiştirilmez.
import { loadConnections } from './connections.ts'
import { loadHooks } from './hooks.ts'
import { loadMcpConfig } from './mcp-client.ts'
import { loadSettings } from './settings.ts'
import { loadG9Settings } from './settings-g9.ts'
import { loadTriggers } from './triggers.ts'

const message = (err: unknown) => (err instanceof Error ? err.message : String(err)).slice(0, 600)

/**
 * root verilirse proje ayarları (.kor/settings*.json) de denetlenir. Uyarı veren yükleyicilerin (hooks) uyarıları
 * da sorun sayılır. Dönen dizi boşsa yapılandırma geçerli.
 */
export async function validateConfigFiles(root: string | null): Promise<string[]> {
  const problems: string[] = []
  const check = async (label: string, run: () => Promise<unknown>) => {
    try {
      await run()
    } catch (err) {
      problems.push(`${label}: ${message(err)}`)
    }
  }
  await Promise.all([
    check('Ayarlar', () => loadSettings(root ?? '.')),
    check('Sağlayıcı ayarları', () => loadG9Settings()),
    check('MCP sunucuları', () => loadMcpConfig()),
    check('Uzak bağlantılar', () => loadConnections()),
    check('Zamanlanmış görevler', () => loadTriggers()),
    check('Kancalar (hooks)', () => loadHooks(root, root !== null, (warning) => problems.push(`Kancalar (hooks): ${warning}`))),
  ])
  return problems
}
