import { Wrench } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import './panel.css'
import { ToolsPanel } from './ToolsPanel.tsx'

export const panel: KorPanel = {
  id: 'tools',
  label: 'Araçlar ve komutlar',
  icon: Wrench,
  keywords: 'araç tool path cli komut satırı ffmpeg docker git yardım help describe tipli tanım niyet intent doğal dil cümle öğret',
  component: ToolsPanel,
}
