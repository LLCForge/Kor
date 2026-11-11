// Etkin MCP bağlantılarının süreç düzeyi sahibi. Ayarlar arayüzü yapılandırmayı değiştirince canlı yeniden kurar
// (tüm bağlantıları kapatıp yenisinden başlatır — sunucular az sayıdadır). Çağrılar sıralanır: eşzamanlı kapat/başlat
// birbirine karışmaz.
import type { McpServerConfig } from './mcp-config.ts'
import { connectMcpServer, listResourcesFrom, readResourceFrom, stopMcp, type McpConnection, type McpRuntime } from './mcp-client.ts'
import { setDynamicTools } from './dynamic-tools.ts'
import { setLiveMcpServers } from './mcp-catalog-live.ts' // [cdx] sistem istemi MCP bölümü
import type { Tool } from './tools.ts'

export interface McpServerStatus {
  id: string
  label: string | null
  transport: 'stdio' | 'http'
  /** Gösterim için komut+argüman ya da URL (gizli env/başlık değerleri burada yer almaz). */
  target: string
  enabled: boolean
  state: 'connected' | 'error' | 'disabled'
  toolCount: number
  error: string | null
}

let runtime: McpRuntime | null = null
let queue: Promise<void> = Promise.resolve()
/** Bağlı sunucuların yapılandırma imzası: aynı kalanlar yeniden başlatılmaz (bir sunucu değişince hepsi düşmesin). */
const liveSig = new Map<string, string>()
const sigOf = (c: McpServerConfig) => JSON.stringify(c)
/** Araç adı → güncel bağlantının çalıştırıcısı. Oturumlar (session.tools) araç nesnesini tutar; yeniden bağlanınca da taze istemciye gitsin. */
const liveRuns = new Map<string, Tool['run']>()

/** Çöken sunucunun araçları düşer; bağlı/yeni bağlantı listesini kayıt defterine ve sistem istemine yansıtır. */
function publish(next: McpRuntime): void {
  runtime = next
  liveRuns.clear()
  for (const tool of next.connections.flatMap((c) => c.tools)) liveRuns.set(tool.name, tool.run)
  next.tools = next.connections.flatMap((c) =>
    c.tools.map((tool): Tool => ({
      ...tool,
      run: (raw, ctx) => {
        const run = liveRuns.get(tool.name)
        if (!run) throw new Error(`MCP sunucusu bağlı değil: ${c.id}. Ayarlar > MCP'den yeniden bağlan.`)
        return run(raw, ctx)
      },
    })),
  )
  setDynamicTools('mcp', next.tools)
  setLiveMcpServers(next.connections.map((c) => ({ id: c.id, tools: c.tools.map((t) => t.name) })))
}

const DROPPED = 'Sunucu bağlantısı koptu'

function watch(conn: McpConnection): void {
  conn.onClosed = () => {
    liveSig.delete(conn.id) // aynı yapılandırma artık "bağlı" sayılmaz: yeniden bağlan gerçekten bağlanır
    if (!runtime?.connections.includes(conn)) return
    console.warn(`MCP sunucusu '${conn.id}' bağlantısı koptu`)
    publish({
      connections: runtime.connections.filter((c) => c !== conn),
      tools: [],
      errors: [...runtime.errors.filter((e) => e.id !== conn.id), { id: conn.id, error: DROPPED }],
    })
  }
}

export async function applyMcp(configs: readonly McpServerConfig[]): Promise<void> {
  queue = queue
    .catch(() => {})
    .then(async () => {
      const wanted = new Map(configs.filter((c) => c.enabled).map((c) => [c.id, c] as const))
      const keep: McpConnection[] = []
      const closing: Promise<void>[] = []
      for (const conn of runtime?.connections ?? []) {
        const cfg = wanted.get(conn.id)
        if (cfg && liveSig.get(conn.id) === sigOf(cfg)) keep.push(conn)
        else {
          conn.onClosed = undefined
          closing.push(conn.close().catch(() => {}))
          liveSig.delete(conn.id)
        }
      }
      await Promise.all(closing)
      const errors: McpRuntime['errors'] = []
      const fresh: McpConnection[] = []
      await Promise.all(
        [...wanted.values()]
          .filter((cfg) => !keep.some((k) => k.id === cfg.id))
          .map(async (cfg) => {
            try {
              const conn = await connectMcpServer(cfg)
              watch(conn)
              fresh.push(conn)
              liveSig.set(cfg.id, sigOf(cfg))
            } catch (err) {
              errors.push({ id: cfg.id, error: err instanceof Error ? err.message : String(err) })
            }
          }),
      )
      // Kurulum sürerken düşen bağlantı listeye geri girmesin.
      const all = [...keep, ...fresh]
      for (const conn of all) if (conn.closed) errors.push({ id: conn.id, error: DROPPED })
      publish({ connections: all.filter((c) => !c.closed), tools: [], errors })
    })
  return queue
}

export async function stopMcpManager(): Promise<void> {
  await queue.catch(() => {})
  const stopping = runtime
  runtime = null
  liveRuns.clear()
  for (const conn of stopping?.connections ?? []) conn.onClosed = undefined
  if (stopping) await stopMcp(stopping)
  liveSig.clear()
  setLiveMcpServers([])
  setDynamicTools('mcp', [])
}

/** Bağlı tüm MCP sunucularının kaynaklarını listeler. Kaynak desteklemeyen sunucular sessizce atlanır. */
export function listMcpResources(): Promise<Array<{ server: string; uri: string; name: string; mimeType?: string }>> {
  return runtime ? listResourcesFrom(runtime.connections) : Promise.resolve([])
}

/** Bir MCP kaynağının içeriğini okur. server verilmezse bağlı sunucularda sırayla aranır. */
export function readMcpResource(uri: string, server?: string): Promise<string> {
  if (!runtime) return Promise.reject(new Error('Bağlı MCP sunucusu yok'))
  return readResourceFrom(runtime.connections, uri, server)
}

/** Verilen yapılandırmayı mevcut çalışma zamanı durumuyla birleştirir (yeniden kurmaz). */
export function mcpStatuses(configs: readonly McpServerConfig[]): McpServerStatus[] {
  return configs.map((c) => {
    const conn = runtime?.connections.find((x) => x.id === c.id)
    const err = runtime?.errors.find((e) => e.id === c.id)
    return {
      id: c.id,
      label: c.label ?? null,
      transport: c.transport,
      target: c.transport === 'stdio' ? [c.command, ...c.args].join(' ') : c.url,
      enabled: c.enabled,
      state: !c.enabled ? 'disabled' : conn ? 'connected' : 'error',
      toolCount: conn?.tools.length ?? 0,
      error: err?.error ?? (c.enabled && !conn ? 'bağlı değil' : null),
    }
  })
}
