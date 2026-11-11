// [g12] Toplu silme onayı: ajan çok sayıda dosya silecek bir komut çalıştırmadan önce dosya listesiyle ikinci, açık onay.
import { TriangleAlert } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import type { BulkDeleteRequest } from '../../ipc-g12.ts'
import { api, errorMessage } from '../api.ts'
import { Modal } from '../NameDialog.tsx'
import { uiLocale } from '../ui-locale.ts'

interface PendingDelete {
  sessionId: string
  id: string
  request: BulkDeleteRequest
}

export function BulkDeleteHost({ activeSessionId }: { activeSessionId: string }) {
  const [queue, setQueue] = useState<PendingDelete[]>([])
  const [error, setError] = useState<string | null>(null)
  const titleId = useId()

  useEffect(
    () =>
      api.onEvent((event) => {
        const { sessionId, ...body } = event
        if (body.type === 'g12:bulkDelete') setQueue((q) => [...q, { sessionId, id: body.id, request: body.request }])
        if (body.type === 'g12:bulkDeleteDone') setQueue((q) => q.filter((p) => p.id !== body.id))
      }),
    [],
  )

  const current = queue[0]
  if (!current) return null
  const { request } = current

  const answer = async (approved: boolean) => {
    try {
      await api.g12AnswerBulkDelete(current.id, approved)
      setQueue((q) => q.filter((p) => p.id !== current.id))
      setError(null)
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  const more = request.total - request.files.length
  return (
    <Modal key={current.id} onClose={() => void answer(false)} labelledBy={titleId}>
      <div className="dialog-body g12-bulk">
        <h2 id={titleId} className="dialog-title">Toplu silme onayı</h2>
        <p className="dialog-desc">
          {current.sessionId === activeSessionId ? 'Ajan' : 'Başka bir sohbetteki ajan'} şu komutla{' '}
          {request.total > 0 ? (
            <strong>
              {request.capped ? 'en az ' : ''}
              {request.total.toLocaleString(uiLocale())} dosya
            </strong>
          ) : (
            'sayısı belirlenemeyen dosyaları'
          )}{' '}
          silmek istiyor. Silinen dosyalar geri alınamaz.
        </p>
        <pre className="approval-subject">{request.command}</pre>
        {request.files.length > 0 && (
          <ul className="g12-file-list" aria-label="Silinecek dosyalar">
            {request.files.map((file) => (
              <li key={file}>{file}</li>
            ))}
            {more > 0 && (
              <li className="muted">
                ... ve {more.toLocaleString(uiLocale())}
                {request.capped ? '+' : ''} dosya daha
              </li>
            )}
          </ul>
        )}
        {request.unresolved.length > 0 && (
          <p className="g12-warning">
            <TriangleAlert size={14} aria-hidden="true" /> Hedefi belirlenemeyen kısımlar (değişken, komut ikamesi vb.): {request.unresolved.join(', ')}
          </p>
        )}
        {error && (
          <p className="error-text" role="alert">
            {error}
          </p>
        )}
        <div className="dialog-actions">
          <span className="muted">{queue.length > 1 ? `${queue.length - 1} onay daha bekliyor` : ''}</span>
          <div className="dialog-actions-right">
            {/* Varsayılan odak reddetmede: Enter ile yanlışlıkla silme olmasın. */}
            <button type="button" className="btn" onClick={() => void answer(false)} autoFocus>
              Reddet
            </button>
            <button type="button" className="btn danger" onClick={() => void answer(true)}>
              Sil
            </button>
          </div>
        </div>
      </div>
    </Modal>
  )
}
