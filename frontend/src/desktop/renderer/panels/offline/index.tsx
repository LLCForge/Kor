import { WifiOff } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { OfflinePanel } from './OfflinePanel.tsx'

export const panel: KorPanel = {
  id: 'offline',
  label: 'Çevrimdışı',
  icon: WifiOff,
  keywords: 'offline çevrimdışı internetsiz air-gap airgap usb paket bundle yerel model llama.cpp ollama lm studio gguf embedding katalog',
  component: OfflinePanel,
}
