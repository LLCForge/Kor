import { Globe } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { WebPanel } from './WebPanel.tsx'

export const panel: KorPanel = {
  id: 'web',
  label: 'Web siteleri',
  icon: Globe,
  keywords: 'web site hosting cpanel plesk directadmin cyberpanel hestia wordpress woocommerce eklenti plugin yönlendirme redirect teklif sözleşme müşteri rapor kvkk e-fatura seo',
  component: WebPanel,
}
