import { useEffect, useId, useState } from 'react'
import { Modal } from './NameDialog.tsx'

export interface ConfirmOptions {
  title: string
  message: string
  confirmLabel: string
  /** Geri alınamayan işlemler: onay düğmesi kırmızı olur. */
  danger?: boolean
}

interface PendingConfirm extends ConfirmOptions {
  resolve: (ok: boolean) => void
}

let showConfirm: ((pending: PendingConfirm) => void) | null = null

/**
 * window.confirm yerine temalı onay penceresi. Odak önce "İptal"e gider, Esc vazgeçer.
 * ConfirmHost takılı değilse işlem yapılmaz (false): yanlışlıkla onaysız silme olmasın.
 */
export function confirmAction(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    if (showConfirm) showConfirm({ ...options, resolve })
    else resolve(false)
  })
}

/** Pencere başına bir kez, App içinde takılır. */
export function ConfirmHost() {
  const [pending, setPending] = useState<PendingConfirm | null>(null)
  // Karar verildi, çıkış animasyonu oynuyor; diyalog kapanınca (onClose) kaldırılır.
  const [closing, setClosing] = useState(false)
  const titleId = useId()

  useEffect(() => {
    showConfirm = (next) =>
      setPending((current) => {
        current?.resolve(false)
        setClosing(false)
        return next
      })
    return () => {
      showConfirm = null
    }
  }, [])

  if (!pending) return null
  // Söz bir kez çözülür (ikinci resolve etkisizdir); İptal/Onay animasyonla, Esc diyalogun kendisinden kapanır.
  const finish = (ok: boolean) => {
    pending.resolve(ok)
    setClosing(true)
  }
  const closed = () => {
    pending.resolve(false)
    setClosing(false)
    setPending(null)
  }

  return (
    <Modal onClose={closed} labelledBy={titleId} closing={closing}>
      <div className="dialog-body">
        <h2 id={titleId} className="dialog-title">{pending.title}</h2>
        <p className="dialog-desc">{pending.message}</p>
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={() => finish(false)}>İptal</button>
            <button type="button" className={`btn ${pending.danger ? 'danger' : 'primary'}`} onClick={() => finish(true)}>
              {pending.confirmLabel}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  )
}
