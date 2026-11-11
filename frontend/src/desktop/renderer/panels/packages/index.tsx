import { Boxes } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import './panel.css'
import { PackagesPanel } from './PackagesPanel.tsx'

export const panel: KorPanel = {
  id: 'packages',
  label: 'Paketler',
  icon: Boxes,
  keywords: 'packages paket monorepo workspace çalışma alanı affected etkilenen deps bağımlılık maliyet',
  component: PackagesPanel,
}
