import { Server } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { RemotePanel } from './RemotePanel.tsx'

export const panel: KorPanel = {
  id: 'remote',
  label: 'Uzak sunucular',
  icon: Server,
  keywords: 'remote uzak ssh sunucu server bağlan connect keşif discover servis service port nginx caddy docker proxmox systemd cron',
  component: RemotePanel,
}
