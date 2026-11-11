import { ScanEye } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { VisualPanel } from './VisualPanel.tsx'

export const panel: KorPanel = {
  id: 'visual',
  label: 'Görsel girdi',
  icon: ScanEye,
  keywords: 'görsel ekran görüntüsü screenshot figma png diyagram diagram mimari harita visualize çağrı grafı veri akışı ocr ses voice whisper mikrofon',
  component: VisualPanel,
}
