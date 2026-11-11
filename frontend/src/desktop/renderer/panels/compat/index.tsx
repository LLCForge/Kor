import { Puzzle } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import { CompatPanel } from './CompatPanel.tsx'
import '../memory/panel.css'

export const panel: KorPanel = {
  id: 'compat',
  label: 'Uyumluluk',
  icon: Puzzle,
  keywords: 'compat uyumluluk cursor claude code windsurf codex copilot kural rules çakışma conflicts mcp export dışa aktar',
  component: CompatPanel,
}
