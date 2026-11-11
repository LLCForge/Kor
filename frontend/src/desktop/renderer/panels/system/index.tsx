import { MonitorCog } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import '../offline/panel.css'
import './panel.css'
import { SystemPanel } from './SystemPanel.tsx'

export const panel: KorPanel = {
  id: 'system',
  label: 'Sistem',
  icon: MonitorCog,
  keywords: 'sistem system sağlık health disk bellek ram cpu gpu vram pil temizlik clean önbellek cache docker bilgisayar kullanımı computer use masaüstü denetimi katman izin pencere düzeni layout monitör',
  component: SystemPanel,
}
