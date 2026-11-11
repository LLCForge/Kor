import { ListChecks } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { RulesPanel } from './RulesPanel.tsx'

export const panel: KorPanel = {
  id: 'rules',
  label: 'Kural uyumu',
  icon: ListChecks,
  keywords: 'kural rules uyum compliance AGENTS.md CLAUDE.md cursor rules ihlal violation denetle check',
  component: RulesPanel,
}
