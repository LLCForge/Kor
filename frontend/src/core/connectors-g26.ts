// [g26] Yerel bağlayıcı kataloğu. Ağdan liste çekilmez; araç allowlist'i daraltır.
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { projectDotDir } from './layout-g28.ts' // [g28]

const connectorSchema = z.object({
  id: z.string().regex(/^[a-z0-9][a-z0-9-]{0,40}$/),
  title: z.string().min(1).max(80),
  tools: z.array(z.string().regex(/^[a-z0-9_]{1,60}$/)).min(1).max(40),
})
export type Connector = z.infer<typeof connectorSchema>

export const CONNECTOR_TEMPLATES: readonly Connector[] = [
  { id: 'workspace-read', title: 'Çalışma dizini okuma', tools: ['read_file', 'grep', 'list_files'] },
  { id: 'workspace-search', title: 'Arama', tools: ['grep', 'project_search', 'file_search'] },
]

export function connectorAllows(connector: Connector, toolName: string): boolean {
  return connector.tools.includes(toolName)
}

export async function loadConnectors(root: string): Promise<Connector[]> {
  const byId = new Map<string, Connector>(CONNECTOR_TEMPLATES.map((item) => [item.id, item]))
  try {
    const parsed = z.array(connectorSchema).max(50).safeParse(JSON.parse(await readFile(path.join(projectDotDir(root), 'connectors.json'), 'utf8')))
    if (parsed.success) for (const item of parsed.data) byId.set(item.id, item)
  } catch {
    // Dosya yoksa şablonlar yeter.
  }
  return [...byId.values()]
}
