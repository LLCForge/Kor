import { BookOpen } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { KnowledgePanel } from './KnowledgePanel.tsx'

export const panel: KorPanel = {
  id: 'knowledge',
  label: 'Bilgi tabanı',
  icon: BookOpen,
  keywords: 'know bilgi tabanı knowledge not note özet digest haftalık weekly eskimiş stale recall',
  component: KnowledgePanel,
}
