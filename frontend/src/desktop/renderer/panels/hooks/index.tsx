import { Anchor } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import { HooksPanel } from './HooksPanel.tsx'

export const panel: KorPanel = {
  id: 'hooks',
  label: "Hook'lar",
  icon: Anchor,
  keywords: 'hook hooks.toml settings.json claude code olay event pre-edit post-edit pre-commit on-task-done on-error kuru çalıştır dry run',
  component: HooksPanel,
}
