import { PackageCheck } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import '../offline/panel.css'
import './panel.css'
import { PrereqsPanel } from './PrereqsPanel.tsx'

export const panel: KorPanel = {
  id: 'prereqs',
  label: 'Sistem gereksinimleri',
  icon: PackageCheck,
  keywords: 'sistem gereksinim kurulum docker podman wsl wsl2 xcode webview2 izin erişilebilirlik ekran kaydı mikrofon winget brew apt dnf pacman zypper apk nix paket yöneticisi prereq',
  component: PrereqsPanel,
}
