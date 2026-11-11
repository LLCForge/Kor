import { Zap } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { TriggersPanel } from './TriggersPanel.tsx'

export const panel: KorPanel = {
  id: 'triggers',
  label: 'Tetikleyiciler',
  icon: Zap,
  keywords: 'tetikleyici trigger daemon webhook cron zamanlama git push log günlük telegram slack e-posta bildirim onay approve pr otomasyon',
  component: TriggersPanel,
}
