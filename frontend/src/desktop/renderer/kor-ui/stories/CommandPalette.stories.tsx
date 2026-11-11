import { action, type StoryDefault } from '@ladle/react'
import { useEffect } from 'react'
import type { PaletteActions } from '../../CommandPalette.tsx'
import type { SessionMeta } from '../../../ipc.ts'
import { CommandPalette } from '../CommandPalette.tsx'

export default { title: 'kor-ui / CommandPalette' } satisfies StoryDefault

const NAMES = [
  'newChat', 'openFolder', 'copyPath', 'openPanel', 'openSettings', 'openSession', 'showShortcuts', 'toggleSidebar',
  'archiveCurrent', 'pinCurrent', 'deleteCurrent', 'duplicateCurrent', 'renameCurrent', 'nextSession', 'prevSession',
  'gitPush', 'gitMerge', 'gitRebase', 'gitPr', 'exportCurrent',
] as const satisfies readonly (keyof PaletteActions)[]

// Hikâyede eylemler Ladle'ın eylem günlüğüne yazılır (backend yok).
const actions = Object.fromEntries(NAMES.map((n) => [n, action(n)])) as unknown as PaletteActions

const sessions = [
  { id: 's1', title: 'auth sistemini refactor et', updatedAt: Date.now() - 60_000 },
  { id: 's2', title: 'README çevirisi', updatedAt: Date.now() - 3_600_000 },
] as unknown as SessionMeta[]

/** Ctrl/Cmd+K ile açılır; hikâye açık başlar. */
export const Open = () => {
  useEffect(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }))
  }, [])
  return <CommandPalette actions={actions} sessions={sessions} />
}

export const Closed = () => (
  <>
    <p>Ctrl+K ile açın.</p>
    <CommandPalette actions={actions} sessions={sessions} />
  </>
)
