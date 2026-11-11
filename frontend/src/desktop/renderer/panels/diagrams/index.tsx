import { Workflow } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { DiagramsPanel } from './DiagramsPanel.tsx'

export const panel: KorPanel = {
  id: 'diagrams',
  label: 'Diyagramlar',
  icon: Workflow,
  keywords: 'diyagram diagram mermaid plantuml d2 senkron sync mimari architecture er sequence deploy check',
  component: DiagramsPanel,
}
