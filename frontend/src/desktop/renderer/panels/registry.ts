// Faz 4 panel kaydı (§12.3): sağ paneldeki ek görünümler. Her panel kendi klasöründe `index.tsx` içinde
// `panel: KorPanel` dışa aktarır ve buraya tek satırla eklenir; SidePanel görünüm menüsü ve komut paleti
// (Görünüm sayfası) bu listeden üretilir.
import type { ComponentType } from 'react'
import { panel as graph } from './graph/index.tsx'
import { panel as packages } from './packages/index.tsx'
import { panel as cost } from './cost/index.tsx'
import { panel as context } from './context/index.tsx'
import { panel as memory } from './memory/index.tsx'
import { panel as compat } from './compat/index.tsx'
import { panel as providers } from './providers/index.tsx'
import { panel as security } from './security/index.tsx'
import { panel as team } from './team/index.tsx'
import { panel as offline } from './offline/index.tsx'
import { panel as tasks } from './tasks/index.tsx'
import { panel as triggers } from './triggers/index.tsx'
import { panel as deploy } from './deploy/index.tsx'
import { panel as replay } from './replay/index.tsx'
import { panel as remote } from './remote/index.tsx'
import { panel as analysis } from './analysis/index.tsx'
import { panel as visual } from './visual/index.tsx'
import { panel as perf } from './perf/index.tsx'
import { panel as plans } from './plans/index.tsx'
import { panel as sessionTree } from './sessions/index.tsx'
import { panel as shellui } from './shellui/index.tsx'
import { panel as web } from './web/index.tsx'
import { panel as tools } from './tools/index.tsx'
import { panel as media } from './media/index.tsx'
import { panel as peers } from './peers/index.tsx'
import { panel as system } from './system/index.tsx'
import { panel as prereqs } from './prereqs/index.tsx'
import { panel as forks } from './forks/index.tsx'
import { panel as diagrams } from './diagrams/index.tsx'
import { panel as knowledge } from './knowledge/index.tsx'
import { panel as rules } from './rules/index.tsx'
import { panel as docs } from './docs/index.tsx'
import { panel as hooks } from './hooks/index.tsx'
import { panel as mcpdev } from './mcpdev/index.tsx'

export interface KorPanelProps {
  /** Etkin sohbet (yoksa null) */
  sessionId: string | null
}

export interface KorPanel {
  /** Benzersiz görünüm kimliği, ör. `graph`; `panel:view` olayı ve palet bu kimliği kullanır. */
  id: string
  /** Türkçe kaynak metin (çeviri kataloglardan gelir) */
  label: string
  icon: ComponentType<{ size?: number }>
  /** Palet araması için ek sözcükler */
  keywords?: string
  component: ComponentType<KorPanelProps>
}

export const PANELS: readonly KorPanel[] = [graph, packages, cost, context, memory, compat, providers, security, team, offline, tasks, triggers, deploy, replay, remote, analysis, visual, perf, plans, sessionTree, shellui, web, tools, media, peers, system, prereqs, forks, diagrams, knowledge, rules, docs, hooks, mcpdev]

export function findPanel(id: string): KorPanel | undefined {
  return PANELS.find((p) => p.id === id)
}
