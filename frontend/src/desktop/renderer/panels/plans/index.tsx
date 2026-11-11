import { ClipboardList } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import { PlansPanel } from './PlansPanel.tsx'

export const panel: KorPanel = {
  id: 'plans',
  label: 'Planlar',
  icon: ClipboardList,
  keywords: 'plan planlar onayla go dur araştır kilit plan modu .kor/plans',
  component: PlansPanel,
}
