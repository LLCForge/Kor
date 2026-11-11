// §27.3 her mesajın eylem çubuğunda "Buradan dallan" (ADR 0063): o mesaja kadarki bağlamla yeni bir
// oturum açılır, ana oturum değişmez; git projesinde dalın kendi çalışma kopyası olur. İsteğe bağlı
// başka bir modelle ("bunu başka modelle de dene"). Açılış App'teki `kor:fork-open` dinleyicisiyle.
import { GitBranch } from 'lucide-react'
import { useId, useState } from 'react'
import { errorMessage } from '../api.ts'
import type { ChatItem } from '../chat-state.ts'
import { forkApi } from '../ek14-api.ts'
import { g2Bridge } from '../g2/use-g2-app.ts'
import { Modal } from '../NameDialog.tsx'
import { toast } from '../kor-ui/Toast.tsx'
import { messageNumber } from './fork-match.ts'
import './fork.css'

export function ForkHere({ item, items }: { item: ChatItem; items: readonly ChatItem[] }) {
  const [open, setOpen] = useState(false)
  const [model, setModel] = useState('')
  const [shared, setShared] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const titleId = useId()
  if (item.kind !== 'user' && item.kind !== 'assistant') return null
  const sessionId = g2Bridge.current?.activeId
  if (!sessionId) return null

  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      const msgs = await forkApi.messages(sessionId)
      const same = items.filter((i) => i.kind === item.kind && i.text.trim() === item.text.trim())
      const at = messageNumber(msgs, item.kind, item.text, Math.max(0, same.findIndex((i) => i.id === item.id)))
      if (at === null) throw new Error('Bu mesaj kayıtlı dökümde bulunamadı (oturum henüz kaydedilmemiş olabilir).')
      const f = await forkApi.create(sessionId, at, model.trim() || null, shared)
      setOpen(false)
      // Çalışma kopyası dallanma anındaki dosyalarla başlar; neyin geri sarıldığı söylenir.
      const rewound = f.rewound ?? []
      const unrestored = f.unrestored ?? []
      const list = (xs: string[]) => `${xs.slice(0, 5).join(', ')}${xs.length > 5 ? ' …' : ''}`
      if (rewound.length > 0) toast(`${rewound.length} dosya dallanma anındaki haline getirildi: ${list(rewound)}`, 'info')
      if (unrestored.length > 0) toast(`Dallanma anına getirilemeyen dosyalar: ${list(unrestored)}`, 'warning')
      if (f.shellAfter) toast('Sonrasında çalışan kabuk komutlarının etkileri geri alınmadı.', 'warning')
      window.dispatchEvent(new CustomEvent('kor:fork-open', { detail: { id: f.id, worktree: f.worktree ?? null } }))
    } catch (e: unknown) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <button type="button" className="action-btn" onClick={() => setOpen(true)} aria-label="Buradan dallan" title="Buradan dallan (yeni oturum, ana oturum değişmez)" data-kor-cmd="fork.create">
        <GitBranch size={14} aria-hidden="true" />
      </button>
      {open && (
        <Modal onClose={() => setOpen(false)} labelledBy={titleId}>
          <div className="dialog-body">
            <h2 id={titleId} className="dialog-title">Buradan dallan</h2>
            <p className="dialog-desc">Bu mesaja kadarki konuşma yeni bir oturuma kopyalanır; ana oturum olduğu gibi kalır. Git projesinde dal kendi çalışma kopyasında yazar.</p>
            <label className="k14-field">
              Başka bir modelle dene (boş: aynı model)
              <input className="text-input" value={model} onChange={(e) => setModel(e.target.value)} placeholder="anthropic/claude-sonnet-4" translate="no" />
            </label>
            <label className="k14-check">
              <input type="checkbox" checked={shared} onChange={(e) => setShared(e.target.checked)} /> Çalışma kopyası açma (proje klasörünü paylaş)
            </label>
            {error && <p className="error-text" role="alert">{error}</p>}
            <div className="dialog-actions">
              <button type="button" className="btn ghost" onClick={() => setOpen(false)}>Vazgeç</button>
              <button type="button" className="btn primary" disabled={busy} onClick={() => void create()} data-kor-cmd="fork.messages">
                <GitBranch size={14} aria-hidden="true" /> {busy ? 'Dallanıyor…' : 'Dallan'}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  )
}
