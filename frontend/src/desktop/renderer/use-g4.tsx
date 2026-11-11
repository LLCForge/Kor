import { History } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState, type Dispatch, type ReactNode, type SetStateAction } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { CheckpointInfo, ExternalStatus, RewindResult } from '../ipc-g4.ts'
import { api } from './api.ts'
import type { ChatAction, ChatItem, ChatState, FileChange } from './chat-state.ts'
import { onAttachRequest } from './g4-shared.ts'
import { RewindDialog } from './RewindDialog.tsx'
import { matchCheckpoints } from './rewind-match.ts'
import type { G4PanelProps } from './SidePanel.tsx'

type UserItem = Extract<ChatItem, { kind: 'user' }>

const FS_KEYS: unknown[] = ['dir', 'file', 'g4:allFiles']
const isFsKey = (key: unknown) => Array.isArray(key) && FS_KEYS.includes(key[0])
const checkpointsKey = (sessionId: string) => ['g4:checkpoints', sessionId] as const

interface UseG4Args {
  activeId: string
  chat: ChatState
  root: string | null
  dispatchStore: (action: { sessionId: string; action: ChatAction }) => void
  setAttachments: Dispatch<SetStateAction<string[]>>
  setDraft: (text: string) => void
}

interface RewindTarget {
  sessionId: string
  itemId: string
  checkpoint: CheckpointInfo
}

/** Grup 4'ün App içindeki durumu: satıra gitme, harici değişiklikler, kontrol noktaları, izleyici. */
export function useG4({ activeId, chat, root, dispatchStore, setAttachments, setDraft }: UseG4Args) {
  const { mutate } = useSWRConfig()
  const [line, setLineState] = useState<{ line: number | null; seq: number }>({ line: null, seq: 0 })
  const [external, setExternal] = useState<Readonly<Record<string, Readonly<Record<string, ExternalStatus>>>>>({})
  const [rewind, setRewind] = useState<RewindTarget | null>(null)
  const hasMessages = chat.items.length > 0
  const { data: checkpoints } = useSWR(hasMessages ? checkpointsKey(activeId) : null, () => api.g4.listCheckpoints(activeId), {
    revalidateOnFocus: false,
  })

  useEffect(() => {
    if (!root) return
    api.g4.watch().catch((err: unknown) => console.error('Klasör izleyicisi başlatılamadı:', err))
  }, [root])

  useEffect(
    () =>
      api.g4.onEvent((event) => {
        if (event.type === 'fs') {
          void mutate(isFsKey)
          return
        }
        setExternal((prev) => {
          const forSession = { ...prev[event.sessionId] }
          if (event.status) forSession[event.path] = event.status
          else delete forSession[event.path]
          return { ...prev, [event.sessionId]: forSession }
        })
      }),
    [mutate],
  )

  // Kontrol noktası tur başında açılır, dosya listesi tur boyunca dolar.
  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.type === 'turnStart' || event.type === 'done' || event.type === 'error') void mutate(checkpointsKey(event.sessionId))
      }),
    [mutate],
  )

  useEffect(() => onAttachRequest((path) => setAttachments((prev) => (prev.includes(path) ? prev : [...prev, path]))), [setAttachments])

  const checkpointFor = useMemo(() => matchCheckpoints(chat.items, checkpoints ?? []), [chat.items, checkpoints])

  const setLine = useCallback((value?: number) => setLineState((prev) => ({ line: value ?? null, seq: prev.seq + 1 })), [])

  const panel = (openFile: (path: string, line?: number) => void): G4PanelProps => ({
    line: line.line,
    lineSeq: line.seq,
    external: external[activeId] ?? {},
    onOpenFile: openFile,
  })

  const userActions = (item: UserItem): ReactNode => {
    const checkpoint = checkpointFor.get(item.id)
    if (!checkpoint) return null
    const title = chat.running
      ? 'Sohbet çalışırken geri sarılamaz'
      : checkpoint.available
        ? 'Dosyaları bu mesaj gönderilmeden önceki haline döndür'
        : 'Bellek sınırı nedeniyle bu noktaya dönülemiyor'
    return (
      <button
        type="button"
        className="action-btn g4-rewind-btn"
        onClick={() => setRewind({ sessionId: activeId, itemId: item.id, checkpoint })}
        disabled={chat.running || !checkpoint.available}
        title={title}
      >
        <History size={14} aria-hidden="true" />
        <span>Buraya geri sar</span>
      </button>
    )
  }

  const onRewound = (target: RewindTarget, result: RewindResult, cut: boolean) => {
    const changes: FileChange[] = result.changes.map((c) => ({ path: c.path, before: c.before, after: c.after ?? '' }))
    dispatchStore({ sessionId: target.sessionId, action: { type: 'rewound', changes, cutFromItemId: cut && result.conversationCut ? target.itemId : null } })
    if (cut && result.conversationCut) setDraft(target.checkpoint.text)
    void mutate(isFsKey)
    void mutate(checkpointsKey(target.sessionId))
    void mutate('sessions')
  }

  const dialogs = rewind && (
    <RewindDialog
      sessionId={rewind.sessionId}
      checkpointId={rewind.checkpoint.id}
      text={rewind.checkpoint.text}
      onClose={() => setRewind(null)}
      onDone={(result, cut) => onRewound(rewind, result, cut)}
    />
  )

  return { setLine, panel, userActions, dialogs }
}
