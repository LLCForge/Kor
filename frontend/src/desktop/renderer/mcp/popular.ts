// Ayarlar > Geliştirici > MCP mağazası: hazır sunucu kataloğu. Tek tıkla mcp.json'a eklenir ve ETKİN gelir (bağlantı
// hemen kurulur; yerel sunucular npx ile başlar). Anahtar isteyen sunucular önce kısa bir form açar.
// Logolar ayrı modülde (popular-logos.tsx): bu dosya resim içe aktarmaz, node:test ile doğrudan test edilir.
import type { McpServerConfig } from '../../../core/mcp-config.ts'

export type McpCategory = 'gelistirme' | 'web' | 'tarayici' | 'veri' | 'uretkenlik' | 'tasarim' | 'analiz'
export const MCP_CATEGORIES: ReadonlyArray<{ id: McpCategory | 'all'; label: string }> = [
  { id: 'all', label: 'Tümü' },
  { id: 'gelistirme', label: 'Geliştirme' },
  { id: 'web', label: 'Web & arama' },
  { id: 'tarayici', label: 'Tarayıcı' },
  { id: 'veri', label: 'Veri' },
  { id: 'uretkenlik', label: 'Üretkenlik' },
  { id: 'tasarim', label: 'Tasarım' },
  { id: 'analiz', label: 'Analiz' },
]

export interface PopularInput {
  /** Ortam değişkeni adı (env) ya da argüman yer tutucusu */
  key: string
  label: string
  placeholder?: string
  secret?: boolean
  /** 'env' (varsayılan): ortam değişkeni; 'arg': argüman listesindeki `{key}` yerine yazılır */
  target?: 'env' | 'arg'
}

export interface PopularMcpServer {
  id: string
  name: string
  description: string
  /** Logo anahtarı (popular-logos.tsx) ya da kısa metin rozeti */
  logo: string
  category: McpCategory
  /** Platformdan bağımsız tanım: stdio için npx komutu, http için adres */
  stdio?: { pkg: string; args?: string[] }
  http?: { url: string }
  /** Kullanıcıdan alınacak anahtarlar (boş bırakılırsa sunucu bağlanamaz) */
  inputs?: PopularInput[]
  /** Uzak sunucu tarayıcıda OAuth ister; ilk bağlantıda yetkilendirme gerekebilir */
  oauth?: boolean
  /** Öne çıkan (mağazada ilk sırada) */
  featured?: boolean
}

export const POPULAR_MCP_SERVERS: readonly PopularMcpServer[] = [
  // ---- Geliştirme ----
  { id: 'rea', name: 'rea — Tersine mühendislik', description: 'Binary, uygulama ve çalışma zamanı tersine mühendisliği: disassembly, string, sembol ve protokol analizi.', logo: 'REA', category: 'analiz', featured: true, stdio: { pkg: 'rea-agents@6.8.0', args: ['mcp'] } },
  { id: 'context7', name: 'Context7', description: 'Kütüphane ve çerçeve dokümantasyonunu güncel sürümüyle modele verir; eski API hatalarını azaltır.', logo: 'context7', category: 'gelistirme', featured: true, stdio: { pkg: '@upstash/context7-mcp' } },
  { id: 'github', name: 'GitHub', description: 'Depo, issue, pull request ve dosya işlemleri (kişisel erişim jetonuyla).', logo: 'github', category: 'gelistirme', featured: true, stdio: { pkg: '@modelcontextprotocol/server-github' }, inputs: [{ key: 'GITHUB_PERSONAL_ACCESS_TOKEN', label: 'GitHub kişisel erişim jetonu', placeholder: 'ghp_…', secret: true }] },
  { id: 'memory', name: 'Memory (bilgi grafiği)', description: 'Sohbetler arası kalıcı bilgi grafiği: varlıklar, ilişkiler ve gözlemler.', logo: 'MEM', category: 'gelistirme', stdio: { pkg: '@modelcontextprotocol/server-memory' } },
  { id: 'sequential_thinking', name: 'Sequential Thinking', description: 'Adım adım, kendini düzelten düşünce zinciri aracı; karmaşık planlama için.', logo: 'SEQ', category: 'gelistirme', stdio: { pkg: '@modelcontextprotocol/server-sequential-thinking' } },
  { id: 'dev_manager', name: 'Dev Manager', description: 'Geliştirme sunucularını başlatır, durdurur ve günlüklerini okur.', logo: 'dev_manager', category: 'gelistirme', stdio: { pkg: 'dev-manager-mcp' } },
  { id: 'shadcn', name: 'shadcn/ui', description: 'shadcn bileşen kayıtlarını arar, bileşen ve blokları projeye ekler.', logo: 'shadcnui', category: 'gelistirme', stdio: { pkg: 'shadcn@latest', args: ['mcp'] } },
  { id: 'vibe_kanban', name: 'Vibe Kanban', description: 'Görev panosu: ajan görevleri oluşturur, günceller ve durumunu izler.', logo: 'VK', category: 'uretkenlik', stdio: { pkg: 'vibe-kanban', args: ['--mcp'] } },
  { id: 'sentry', name: 'Sentry', description: 'Hata ve performans olaylarını sorgular, issue ayrıntılarını getirir (uzak, OAuth).', logo: 'sentry', category: 'gelistirme', http: { url: 'https://mcp.sentry.dev/mcp' }, oauth: true },
  { id: 'vercel', name: 'Vercel', description: 'Proje, dağıtım ve günlük bilgileri; Vercel dokümantasyonu (uzak, OAuth).', logo: 'vercel', category: 'gelistirme', http: { url: 'https://mcp.vercel.com' }, oauth: true },
  { id: 'cloudflare_docs', name: 'Cloudflare Docs', description: 'Workers, Pages, R2, D1 ve diğer Cloudflare ürünlerinin dokümantasyonu (uzak, anahtar gerekmez).', logo: 'cloudflare', category: 'gelistirme', http: { url: 'https://docs.mcp.cloudflare.com/mcp' } },
  { id: 'huggingface', name: 'Hugging Face', description: 'Model, veri seti ve Space araması; model kartları ve Hub araçları (uzak).', logo: 'huggingface', category: 'gelistirme', http: { url: 'https://huggingface.co/mcp' } },
  { id: 'deepwiki', name: 'DeepWiki', description: 'Herhangi bir GitHub deposunun yapay zekâ ile üretilmiş dokümantasyonunu okur ve sorular sorar (uzak, anahtar gerekmez).', logo: 'DW', category: 'gelistirme', http: { url: 'https://mcp.deepwiki.com/mcp' } },
  // ---- Web & arama ----
  { id: 'exa', name: 'Exa', description: 'Yapay zekâ için web araması ve sayfa içeriği getirme (uzak HTTP sunucusu).', logo: 'exa', category: 'web', featured: true, http: { url: 'https://mcp.exa.ai/mcp' } },
  { id: 'brave_search', name: 'Brave Search', description: 'Web ve yerel işletme araması; gizlilik odaklı arama API\'si.', logo: 'brave', category: 'web', stdio: { pkg: '@modelcontextprotocol/server-brave-search' }, inputs: [{ key: 'BRAVE_API_KEY', label: 'Brave Search API anahtarı', secret: true }] },
  { id: 'tavily', name: 'Tavily', description: 'Ajanlar için arama, içerik çıkarma ve site tarama.', logo: 'TAV', category: 'web', stdio: { pkg: 'tavily-mcp@latest' }, inputs: [{ key: 'TAVILY_API_KEY', label: 'Tavily API anahtarı', placeholder: 'tvly-…', secret: true }] },
  { id: 'firecrawl', name: 'Firecrawl', description: 'Web sayfalarını temiz Markdown\'a çevirir; site tarama, arama ve yapılandırılmış veri çıkarma.', logo: 'FC', category: 'web', stdio: { pkg: 'firecrawl-mcp' }, inputs: [{ key: 'FIRECRAWL_API_KEY', label: 'Firecrawl API anahtarı', placeholder: 'fc-…', secret: true }] },
  { id: 'google_maps', name: 'Google Maps', description: 'Yer arama, adres çözümleme, yol tarifi ve mesafe hesaplama.', logo: 'googlemaps', category: 'web', stdio: { pkg: '@modelcontextprotocol/server-google-maps' }, inputs: [{ key: 'GOOGLE_MAPS_API_KEY', label: 'Google Maps API anahtarı', secret: true }] },
  // ---- Tarayıcı ----
  { id: 'playwright', name: 'Playwright', description: 'Tarayıcı otomasyonu: sayfa açar, tıklar, form doldurur, ekran görüntüsü alır.', logo: 'playwright', category: 'tarayici', featured: true, stdio: { pkg: '@playwright/mcp@latest' } },
  { id: 'chrome_devtools', name: 'Chrome DevTools', description: 'Canlı Chrome oturumunu denetler: konsol, ağ, performans ve DOM.', logo: 'chrome_devtools', category: 'tarayici', stdio: { pkg: 'chrome-devtools-mcp@latest' } },
  { id: 'puppeteer', name: 'Puppeteer', description: 'Headless Chrome ile gezinme, ekran görüntüsü ve sayfa içi JavaScript çalıştırma.', logo: 'puppeteer', category: 'tarayici', stdio: { pkg: '@modelcontextprotocol/server-puppeteer' } },
  // ---- Veri ----
  { id: 'postgres', name: 'PostgreSQL', description: 'Salt okunur SQL sorguları ve şema keşfi (bağlantı dizesiyle).', logo: 'postgresql', category: 'veri', stdio: { pkg: '@modelcontextprotocol/server-postgres', args: ['{DATABASE_URL}'] }, inputs: [{ key: 'DATABASE_URL', label: 'Bağlantı dizesi', placeholder: 'postgresql://kullanici:parola@localhost:5432/veritabani', target: 'arg' }] },
  { id: 'mongodb', name: 'MongoDB', description: 'Koleksiyon sorgulama, toplama ve şema inceleme.', logo: 'mongodb', category: 'veri', stdio: { pkg: 'mongodb-mcp-server' }, inputs: [{ key: 'MDB_MCP_CONNECTION_STRING', label: 'Bağlantı dizesi', placeholder: 'mongodb://localhost:27017', secret: true }] },
  { id: 'supabase', name: 'Supabase', description: 'Proje, tablo, SQL ve Edge Function yönetimi.', logo: 'supabase', category: 'veri', stdio: { pkg: '@supabase/mcp-server-supabase@latest' }, inputs: [{ key: 'SUPABASE_ACCESS_TOKEN', label: 'Supabase kişisel erişim jetonu', placeholder: 'sbp_…', secret: true }] },
  { id: 'airtable', name: 'Airtable', description: 'Base, tablo ve kayıtları okuma/yazma.', logo: 'airtable', category: 'veri', stdio: { pkg: 'airtable-mcp-server' }, inputs: [{ key: 'AIRTABLE_API_KEY', label: 'Airtable kişisel erişim jetonu', placeholder: 'pat…', secret: true }] },
  { id: 'stripe', name: 'Stripe', description: 'Müşteri, ürün, fiyat, ödeme ve abonelik işlemleri.', logo: 'stripe', category: 'veri', stdio: { pkg: '@stripe/mcp', args: ['--tools=all'] }, inputs: [{ key: 'STRIPE_SECRET_KEY', label: 'Stripe gizli anahtarı', placeholder: 'sk_test_…', secret: true }] },
  // ---- Üretkenlik ----
  { id: 'notion', name: 'Notion', description: 'Sayfa ve veritabanlarını arar, okur, oluşturur ve günceller.', logo: 'notion', category: 'uretkenlik', stdio: { pkg: '@notionhq/notion-mcp-server' }, inputs: [{ key: 'NOTION_TOKEN', label: 'Notion entegrasyon jetonu', placeholder: 'ntn_…', secret: true }] },
  { id: 'linear', name: 'Linear', description: 'Issue, proje ve döngü yönetimi (uzak, OAuth).', logo: 'linear', category: 'uretkenlik', http: { url: 'https://mcp.linear.app/mcp' }, oauth: true },
  { id: 'atlassian', name: 'Jira & Confluence', description: 'Atlassian issue, sayfa ve proje işlemleri (uzak, OAuth).', logo: 'jira', category: 'uretkenlik', http: { url: 'https://mcp.atlassian.com/v1/mcp' }, oauth: true },
  { id: 'slack', name: 'Slack', description: 'Kanal listeleme, mesaj okuma/gönderme ve tepki ekleme (bot jetonuyla).', logo: 'SLK', category: 'uretkenlik', stdio: { pkg: '@modelcontextprotocol/server-slack' }, inputs: [{ key: 'SLACK_BOT_TOKEN', label: 'Slack bot jetonu', placeholder: 'xoxb-…', secret: true }, { key: 'SLACK_TEAM_ID', label: 'Çalışma alanı kimliği', placeholder: 'T0…' }] },
  { id: 'todoist', name: 'Todoist', description: 'Görev oluşturma, listeleme, tamamlama ve güncelleme.', logo: 'todoist', category: 'uretkenlik', stdio: { pkg: '@abhiz123/todoist-mcp-server' }, inputs: [{ key: 'TODOIST_API_TOKEN', label: 'Todoist API jetonu', secret: true }] },
  // ---- Tasarım ----
  { id: 'figma', name: 'Figma', description: 'Figma dosyalarından düzen, stil ve bileşen verisini okur; tasarımdan koda.', logo: 'figma', category: 'tasarim', stdio: { pkg: 'figma-developer-mcp', args: ['--stdio'] }, inputs: [{ key: 'FIGMA_API_KEY', label: 'Figma kişisel erişim jetonu', placeholder: 'figd_…', secret: true }] },
]

const isWindows = () => typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent)

/** Kart tanımını mcp.json kaydına çevirir: ETKİN gelir. Windows'ta npx.cmd doğrudan başlatılamaz: cmd /c üzerinden. */
export function popularToConfig(server: PopularMcpServer, values: Readonly<Record<string, string>> = {}, windows = isWindows()): McpServerConfig {
  const common = { id: server.id, label: server.name, enabled: true }
  if (server.http) return { transport: 'http', ...common, url: server.http.url, headers: {} }
  const env: Record<string, string> = {}
  const argValues: Record<string, string> = {}
  for (const input of server.inputs ?? []) {
    const v = (values[input.key] ?? '').trim()
    if (!v) continue
    if (input.target === 'arg') argValues[input.key] = v
    else env[input.key] = v
  }
  const rawArgs = server.stdio!.args ?? []
  const args = rawArgs.map((a) => a.replace(/\{([A-Z0-9_]+)\}/g, (_m, k: string) => argValues[k] ?? '')).filter((a) => a !== '')
  const argv = ['-y', server.stdio!.pkg, ...args]
  return windows
    ? { transport: 'stdio', ...common, command: 'cmd', args: ['/d', '/s', '/c', 'npx', ...argv], env }
    : { transport: 'stdio', ...common, command: 'npx', args: argv, env }
}

/** Mağaza araması: ad, açıklama, kimlik ve kategori etiketi üzerinde küçük/büyük harf duyarsız. */
export function filterPopular(list: readonly PopularMcpServer[], query: string, category: McpCategory | 'all'): PopularMcpServer[] {
  const q = query.trim().toLocaleLowerCase('tr')
  return list.filter((s) => (category === 'all' || s.category === category) && (!q || `${s.name} ${s.description} ${s.id}`.toLocaleLowerCase('tr').includes(q)))
}
