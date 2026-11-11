import { PlugZap } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { McpDevPanel } from './McpDevPanel.tsx'

export const panel: KorPanel = {
  id: 'mcpdev',
  label: 'MCP geliştirici',
  icon: PlugZap,
  keywords: 'mcp dev geliştirici inspector denetleyici sunucu server araç tool json-rpc reload yeniden yükle',
  component: McpDevPanel,
}
