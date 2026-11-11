import { Users } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { TeamPanel } from './TeamPanel.tsx'

export const panel: KorPanel = {
  id: 'team',
  label: 'Takım',
  icon: Users,
  keywords: 'team takım sunucu server üye member paylaşılan kural rules',
  component: TeamPanel,
}
