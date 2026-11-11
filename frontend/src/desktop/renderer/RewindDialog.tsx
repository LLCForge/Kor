import { AlertTriangle } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { RewindAction, RewindResult } from '../ipc-g4.ts'
import { api, errorMessage } from './api.ts'
import { Modal } from './NameDialog.tsx'

const ACTION_LABEL: Record<RewindAction, string> = {
  restore: 'eski haline döner',
  delete: 'silinir (o anda yoktu)',
  create: 'yeniden oluşturulur',
}

interface RewindDialogProps {
  sessionId: string
  checkpointId: string
  text: string
  onClose: () => void
  onDone: (result: RewindResult, cutConversation: boolean) => void
}

/** "Buraya geri sar" onayı: hangi dosyaların değişeceğini listeler; konuşmayı kesmek isteğe bağlıdır. */
export function RewindDialog({ sessionId, checkpointId, text, onClose, onDone }: RewindDialogProps) {
  const titleId = useId()
  const cutId = useId()
  const [cut, setCut] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const { data, error, isLoading } = useSWR(['g4:rewindPreview', sessionId, checkpointId], () => api.g4.previewRewind(sessionId, checkpointId), {
    revalidateOnFocus: false,
  })
  const short = text.length > 140 ? `${text.slice(0, 139)}…` : text
  const nothingToDo = data !== undefined && data.files.length === 0 && !cut

  const confirm = async () => {
    setBusy(true)
    setFailure(null)
    try {
      const result = await api.g4.rewind(sessionId, checkpointId, cut)
      onDone(result, cut)
      onClose()
    } catch (err) {
      setFailure(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <div className="dialog-body">
        <h2 id={titleId} className="dialog-title">Buraya geri sar</h2>
        <p className="dialog-desc text-pretty">
          Dosyalar şu mesaj gönderilmeden önceki haline döner: <q className="g4-quote">{short}</q>
        </p>
        {isLoading && <p className="muted">Değişecek dosyalar hesaplanıyor...</p>}
        {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
        {data && (
          <>
            {data.files.length === 0 ? (
              <p className="muted">Dosyalarda değişecek bir şey yok; diskteki haller zaten o andaki gibi.</p>
            ) : (
              <>
                <p className="g4-meta">{data.files.length} dosya değişecek:</p>
                <ul className="g4-rewind-list">
                  {data.files.map((f) => (
                    <li key={f.path}>
                      <code>{f.path}</code> <span className="muted">{ACTION_LABEL[f.action]}</span>
                      {f.external && (
                        <span className="g4-warn g4-inline">
                          <AlertTriangle size={13} aria-hidden="true" /> harici değişiklikler de kaybolur
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
              </>
            )}
            {data.running && <p className="error-text" role="alert">Sohbet çalışıyor; geri sarmak için önce durdur.</p>}
          </>
        )}
        <label htmlFor={cutId} className="g4-check">
          <input id={cutId} type="checkbox" checked={cut} onChange={(e) => setCut(e.target.checked)} />
          <span>Konuşmayı da bu mesajdan itibaren sil (mesaj metni yazma kutusuna geri konur)</span>
        </label>
        {failure && <p className="error-text" role="alert">{failure}</p>}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose} disabled={busy}>İptal</button>
            <button type="button" className="btn primary" onClick={() => void confirm()} disabled={busy || !data || data.running || nothingToDo}>
              {busy ? 'Geri sarılıyor...' : 'Geri sar'}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  )
}
