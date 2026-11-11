import { FileText } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { DocsPanel } from './DocsPanel.tsx'

export const panel: KorPanel = {
  id: 'docs',
  label: 'Dokümanlar',
  icon: FileText,
  keywords: 'doküman docs pdf docx confluence notion wiki intranet indeks index spec belge',
  component: DocsPanel,
}
