// Çalışma zamanında eklenen araçların (MCP sunucuları, dosyadan tanımlı yerel araçlar, ileride eklenti araçları)
// süreç düzeyi kayıt defteri. Kaynağa göre tutulur; createSession hepsini çekirdek araçlara ekler. Ayrı modül:
// tools.ts ile araç kaynakları arasında döngüsel import olmasın — yalnızca Tool tipine bağlıdır.
import type { Tool } from './tools.ts'

const bySource = new Map<string, readonly Tool[]>()

/** Bir kaynağın araçlarını ayarlar (boş dizi kaynağı kaldırır). Kaynak: 'mcp', 'file', ... */
export function setDynamicTools(source: string, tools: readonly Tool[]): void {
  if (tools.length > 0) bySource.set(source, tools)
  else bySource.delete(source)
}

/** Her isteğe giden dinamik araçlar. MCP hariç: tek sunucu (ör. rea, 229 araç) ~75 bin token tutabiliyor. */
export function getDynamicTools(): Tool[] {
  return [...bySource].flatMap(([source, list]) => (source === 'mcp' ? [] : list))
}

/** tool_search veya adıyla doğrudan çağrıyla yüklenen MCP araçları. */
export function getDeferredDynamicTools(): readonly Tool[] {
  return bySource.get('mcp') ?? []
}
