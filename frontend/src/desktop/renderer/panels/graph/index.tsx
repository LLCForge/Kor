import { Network } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import { GraphPanel } from './GraphPanel.tsx'
import './panel.css'
// Oturum değişikliklerini panel açılmadan önce toplamaya başlar (bkz. session-changes.ts).
import './session-changes.ts'

export const panel: KorPanel = {
  id: 'graph',
  label: 'Proje grafı',
  icon: Network,
  keywords: 'graph graf kor graph düğüm topluluk hotspot path impact export diff çağıranlar',
  component: GraphPanel,
}
