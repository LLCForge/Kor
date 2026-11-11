import { Network } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import '../remote/panel.css'
import './panel.css'
import { PeersPanel } from './PeersPanel.tsx'

export const panel: KorPanel = {
  id: 'peers',
  label: 'Makineler',
  icon: Network,
  keywords: 'peer makine eşleştir pair devret delegate görev sunucu laptop hafıza sync eşitle yetki acp zed editör handoff en boş',
  component: PeersPanel,
}
