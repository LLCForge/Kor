import { ShieldCheck } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import './panel.css'
import { SecurityPanel } from './SecurityPanel.tsx'

export const panel: KorPanel = {
  id: 'security',
  label: 'Güvenlik',
  icon: ShieldCheck,
  keywords: 'security güvenlik sandbox denetim audit kayıt hash zincir kasa vault sır secret env gizlilik privacy',
  component: SecurityPanel,
}
