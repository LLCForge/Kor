import { Coins } from 'lucide-react'
import './panel.css'
import type { KorPanel } from '../registry.ts'
import { CostPanel } from './CostPanel.tsx'

export const panel: KorPanel = {
  id: 'cost',
  label: 'Maliyet',
  icon: Coins,
  keywords: 'maliyet cost bütçe budget kota quota yönlendirme routing neden why bench dışa aktar csv json',
  component: CostPanel,
}
