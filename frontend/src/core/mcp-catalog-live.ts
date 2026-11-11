// [cdx] Bağlı MCP sunucularının canlı özeti: mcp-manager yazar, prompt.ts okur. Ayrı modül, çünkü prompt.ts'nin
// mcp-manager'ı (→ mcp-client → tools.ts → prompt.ts) import etmesi döngü yaratıyordu.
export interface LiveMcpServer { id: string; tools: string[] }
let live: LiveMcpServer[] = []

export function setLiveMcpServers(servers: LiveMcpServer[]): void {
  live = servers
}

/** Sistem istemi bölümü: bağlı MCP sunucuları ve araç adları — model hangi dış yeteneklerin elinde olduğunu bilsin. */
export function mcpCatalogSection(): string | null {
  if (live.length === 0) return null
  const lines = live.map((c) => `- ${c.id}: ${c.tools.length} tools (${c.tools.slice(0, 12).map((t) => t.replace(/^mcp__[^_]+__/, '')).join(', ')}${c.tools.length > 12 ? ', …' : ''})`)
  return `# MCP SERVERS (connected now)
These external tool servers are live in this session; their tools appear as mcp__<server>__<tool>. Prefer them over re-implementing the same capability with bash when the domain matches (e.g. a reverse-engineering server for binary analysis, a docs server for library APIs, a browser server for live pages). Check them again when the task changes area.
${lines.join('\n')}`
}
