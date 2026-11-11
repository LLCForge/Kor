import { BrainCircuit } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import { MemoryPanel } from './MemoryPanel.tsx'
import './panel.css'

export const panel: KorPanel = {
  id: 'memory',
  label: 'Hafıza',
  icon: BrainCircuit,
  keywords: 'memory hafıza recall forget knowledge bilgi tabanı karar decisions workspace çalışma alanı context bağlı proje git notes',
  component: MemoryPanel,
}
