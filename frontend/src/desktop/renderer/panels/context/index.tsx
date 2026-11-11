import { Gauge } from 'lucide-react'
import '../cost/panel.css'
import type { KorPanel } from '../registry.ts'
import { ContextPanel } from './ContextPanel.tsx'

export const panel: KorPanel = {
  id: 'context',
  label: 'Bağlam',
  icon: Gauge,
  keywords: 'context bağlam token pencere döküm özetleme eşik kor context',
  component: ContextPanel,
}
