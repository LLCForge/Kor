import { Users } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import '../offline/panel.css'
import './panel.css'
import { TasksPanel } from './TasksPanel.tsx'

export const panel: KorPanel = {
  id: 'tasks',
  label: 'Ekip görevleri',
  icon: Users,
  keywords: 'task görev ekip çok ajanlı multi-agent orkestratör orchestrator alt ajan worktree dal branch birleştir merge çakışma conflict',
  component: TasksPanel,
}
