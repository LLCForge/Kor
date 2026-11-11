import { History } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { ReplayPanel } from './ReplayPanel.tsx'

export const panel: KorPanel = {
  id: 'replay',
  label: 'Oturum kayıtları',
  icon: History,
  keywords: 'replay kayıt record oturum session asciinema cast zaman çizelgesi timeline diff dışa aktar export',
  component: ReplayPanel,
}
