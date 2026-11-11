import { CornerDownLeft, Undo2 } from 'lucide-react'
import type { QueuedView } from './use-g2-app.ts'

interface QueueListProps {
  items: readonly QueuedView[]
  onSendNow: (id: string) => void
  onUndo: (id: string) => void
}

/** Ajan çalışırken gönderilen, bir sonraki model çağrısından önce konuşmaya eklenecek mesajlar. */
export function QueueList({ items, onSendNow, onUndo }: QueueListProps) {
  return (
    <ul className="g2-queue" aria-label="Sıradaki mesajlar">
      {items.map((q) => (
        <li key={q.id} className="g2-queued pop-in">
          <span className="badge">Sırada</span>
          <span className="g2-queued-text truncate" title={q.text}>
            {q.text || q.attachments.join(', ')}
          </span>
          <button type="button" className="action-btn" onClick={() => onSendNow(q.id)} aria-label="Turu kesip şimdi gönder" title="Şimdi gönder">
            <CornerDownLeft size={14} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" onClick={() => onUndo(q.id)} aria-label="Geri al: mesajı taslağa döndür" title="Geri al">
            <Undo2 size={14} aria-hidden="true" />
          </button>
        </li>
      ))}
    </ul>
  )
}
