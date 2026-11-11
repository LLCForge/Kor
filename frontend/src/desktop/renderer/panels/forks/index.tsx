import { GitBranch } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { ForksPanel } from './ForksPanel.tsx'

export const panel: KorPanel = {
  id: 'forks',
  label: 'Konuşma dalları',
  icon: GitBranch,
  keywords: 'fork dal dallan branch ağaç tree karşılaştır compare birleştir merge oturum session',
  component: ForksPanel,
}
