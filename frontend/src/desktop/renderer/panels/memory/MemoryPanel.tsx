// Hafıza paneli (§6.1 proje hafızası, §31.3 bilgi tabanı): belgeler, kararlar, bilgi tabanı araması, projeler.
// CLI karşılıkları: kor recall / forget / knowledge export / workspace / context add|remove|projects / memory notes.
import { useCallback, useId, useMemo, useState } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { memoryApi, projmemApi } from './api.ts'
import { DecisionsTab } from './DecisionsTab.tsx'
import { DocsTab } from './DocsTab.tsx'
import { KnowledgeTab } from './KnowledgeTab.tsx'
import { ProjectsTab } from './ProjectsTab.tsx'
import { projectChoices } from './shape.ts'

const TABS = [
  { id: 'docs', label: 'Belgeler' },
  { id: 'decisions', label: 'Kararlar' },
  { id: 'kb', label: 'Bilgi tabanı' },
  { id: 'projects', label: 'Projeler' },
] as const

type Tab = (typeof TABS)[number]['id']

export function MemoryPanel(_props: KorPanelProps) {
  const id = useId()
  const [tab, setTab] = useState<Tab>('docs')
  const overview = useSWR('projmem:overview', () => projmemApi.overview())
  const workspaces = useSWR('memory:workspaces', () => memoryApi.workspaces(), { revalidateOnFocus: false })
  const linked = useSWR(overview.data ? ['memory:projects', overview.data.project] : null, () => memoryApi.projects(), { revalidateOnFocus: false })
  const refreshOverview = useCallback(() => void overview.mutate(), [overview.mutate])
  const choices = useMemo(
    () => projectChoices(overview.data?.project ?? null, workspaces.data?.folders ?? [], linked.data?.projects ?? []),
    [overview.data?.project, workspaces.data, linked.data],
  )

  return (
    <div className="kmem-panel">
      <div className="g14-tabs" role="tablist" aria-label="Hafıza bölümleri">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`${id}-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`${id}-body`}
            className={`g14-tab${tab === t.id ? ' active' : ''}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="g14-body" role="tabpanel" id={`${id}-body`} aria-labelledby={`${id}-${tab}`} data-kor-cmd="projmem.overview">
        {tab === 'kb' ? (
          <KnowledgeTab choices={choices} />
        ) : tab === 'projects' ? (
          <ProjectsTab
            workspaces={workspaces.data}
            linked={linked.data}
            error={workspaces.error ?? linked.error}
            mutateWorkspaces={workspaces.mutate}
            mutateLinked={linked.mutate}
          />
        ) : overview.error ? (
          <p className="error-text" role="alert">{errorMessage(overview.error)}</p>
        ) : !overview.data ? (
          <p className="muted">Yükleniyor...</p>
        ) : tab === 'docs' ? (
          <DocsTab overview={overview.data} onChanged={refreshOverview} />
        ) : (
          <DecisionsTab project={overview.data.project} gitNotes={overview.data.gitNotes} onChanged={refreshOverview} />
        )}
      </div>
    </div>
  )
}
