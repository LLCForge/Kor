import { ScanSearch } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { AnalysisPanel } from './AnalysisPanel.tsx'

export const panel: KorPanel = {
  id: 'analysis',
  label: 'Analiz',
  icon: ScanSearch,
  keywords: 'analiz analysis audit denetim güvenlik security kvkk gdpr uyum compliance pii sarif secret sır enjeksiyon injection',
  component: AnalysisPanel,
}
