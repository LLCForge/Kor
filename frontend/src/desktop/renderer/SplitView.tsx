import { X } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { PermissionMode } from '../../core/permissions.ts'
import { api, errorMessage } from './api.ts'
import type { ChatAction, ChatState } from './chat-state.ts'
import { Composer } from './Composer.tsx'
import type { G3PaneActions } from './g3-bridge.ts'
import { Thread } from './Thread.tsx'
import { approveWithFiles } from './review/review-api.ts' // [review]

interface SplitPaneProps {
  sessionId: string
  chat: ChatState
  usageRefreshKey: number
  dispatch: (action: ChatAction) => void
  onClose: () => void
}

/**
 * Yan yana görünümün ikinci sütunu: kendi sohbetini kendi sessionId'siyle gönderir.
 * Model ve izin modu ana süreçteki etkin sohbetten bağımsızdır (g1:session:config).
 */
export function SplitPane({ sessionId, chat, usageRefreshKey, dispatch, onClose }: SplitPaneProps) {
  // Aynı SWR anahtarları App ile paylaşılır: ek istek yapılmaz.
  const { data: appState, mutate: refreshState } = useSWR('state', () => api.getState())
  const { data: models, error: modelsError } = useSWR('models', () => api.listModels(), { revalidateOnFocus: false })
  const { data: sessions } = useSWR('sessions', () => api.listSessions())
  const { data: config, mutate: refreshConfig } = useSWR(['g1:config', sessionId], () => api.getSessionConfig(sessionId))
  const [draft, setDraft] = useState('')
  const [attachments, setAttachments] = useState<string[]>([])

  const meta = sessions?.find((s) => s.id === sessionId)
  const title = meta?.title ?? 'Sohbet'
  // Dosya seçici açık klasörden göreli yol döner; bu sohbetin klasörü farklıysa yanlış dosya eklenmesin.
  const sameRoot = meta !== undefined && meta.root === appState?.root
  // Yalnızca gönderim hatası turu kapatır; diğer hatalar çalışan turu "bitti" göstermemeli.
  const report = (err: unknown) => dispatch({ type: 'notify', message: errorMessage(err) })

  const sendText = (text: string, files: string[]) => {
    dispatch({ type: 'userSent', text, attachments: files, at: Date.now() })
    api.send(sessionId, text, files).catch((err: unknown) => dispatch({ type: 'failed', message: errorMessage(err) }))
  }

  const send = (text: string) => {
    sendText(text, attachments)
    setDraft('')
    setAttachments([])
  }

  const pane: G3PaneActions = {
    send: (text, files = []) => sendText(text, files),
    answer: async (callId, answer) => {
      await api.g3Answer(sessionId, callId, answer)
      if (answer.kind === 'planApprove') await refreshConfig()
    },
    times: chat.g3?.times ?? {},
  }

  const setConfig = (change: { model?: string; permissionMode?: PermissionMode }) =>
    api
      .setSessionConfig(sessionId, change)
      .then(() => refreshConfig())
      .catch(report)

  const pickFiles = () => {
    if (!sameRoot) {
      report(new Error('Bu sohbetin proje klasörü açık klasörden farklı; dosya eklemek için sohbeti ana bölmede aç.'))
      return
    }
    api
      .pickFiles()
      .then((paths) => setAttachments((prev) => [...new Set([...prev, ...paths])]))
      .catch(report)
  }

  const composer = (
    <Composer
      draft={draft}
      onDraftChange={setDraft}
      running={chat.running}
      disabled={!config}
      attachments={attachments}
      onPickFiles={pickFiles}
      onRemoveAttachment={(path) => setAttachments((prev) => prev.filter((p) => p !== path))}
      onSend={send}
      onAbort={() => void api.abort(sessionId).catch(report)}
      model={config?.model ?? appState?.model ?? ''}
      models={models}
      modelsError={modelsError ? errorMessage(modelsError) : null}
      onModelChange={(model) => void setConfig({ model })}
      permissionMode={config?.permissionMode ?? 'default'}
      defaultPermissionMode={appState?.defaultPermissionMode ?? 'default'}
      onPermissionModeChange={(permissionMode) => void setConfig({ permissionMode })}
      onMakeDefaultMode={(mode) =>
        void api
          .setDefaultPermissionMode(mode)
          .then(() => setConfig({ permissionMode: mode }))
          .then(() => refreshState())
          .catch(report)
      }
      usageRefreshKey={usageRefreshKey}
      sessionId={sessionId}
      root={appState?.root ?? null}
      recentWorkspaces={appState?.recentWorkspaces ?? []}
      onOpenFolder={() => void api.openWorkspace().then(() => refreshState()).catch(report)}
      onClearFolder={() => void api.clearWorkspace().then(() => refreshState()).catch(report)}
      onPickRecentFolder={(path) => void api.setWorkspacePath(path).then(() => refreshState()).catch(report)}
    />
  )

  return (
    <section className="main g1-split-pane" aria-label={`Yan bölme: ${title}`}>
      <header className="main-header">
        <span className="g1-split-title truncate" title={title}>
          {title}
        </span>
        <button type="button" className="action-btn" onClick={onClose} aria-label="Yan bölmeyi kapat" title="Yan bölmeyi kapat">
          <X size={16} aria-hidden="true" />
        </button>
      </header>
      <Thread
        key={sessionId}
        sessionId={sessionId}
        items={chat.items}
        running={chat.running}
        turnStartedAt={chat.turnStartedAt}
        composer={composer}
        pane={pane}
        onApprove={(id, approval, files) => {
          dispatch({ type: 'approved', id })
          void approveWithFiles(id, approval, files).catch(report) // [review]
        }}
      />
    </section>
  )
}
