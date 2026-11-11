import { FolderTree } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import { SessionTreePanel } from './SessionTreePanel.tsx'

export const panel: KorPanel = {
  id: 'sessionTree',
  label: 'Oturum ağacı',
  icon: FolderTree,
  keywords: 'oturum sohbet klasör etiket tag ara arama pin sabitle arşiv renk dışa aktar içe aktar markdown',
  component: SessionTreePanel,
}
