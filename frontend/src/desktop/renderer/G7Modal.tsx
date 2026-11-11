import { X } from 'lucide-react'
import { useEffect, useRef, type ReactNode } from 'react'

interface G7ModalProps {
  open: boolean
  onClose: () => void
  titleId: string
  title: string
  className?: string
  /** Başlık satırının sağına eklenecek düğmeler */
  actions?: ReactNode
  children: ReactNode
}

/** Yerel <dialog>: odak hapsi, Esc ve arka plan tarayıcıdan gelir; başlık + kapat düğmesi ortak. */
export function G7Modal({ open, onClose, titleId, title, className, actions, children }: G7ModalProps) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (open && !dialog.open) dialog.showModal()
    if (!open && dialog.open) dialog.close()
  }, [open])

  return (
    <dialog ref={ref} className={`dialog g7-modal${className ? ` ${className}` : ''}`} onClose={onClose} aria-labelledby={titleId}>
      {open && (
        <>
          <header className="g7-modal-head">
            <h2 id={titleId} className="dialog-title">{title}</h2>
            <div className="g7-modal-actions">
              {actions}
              <button type="button" className="action-btn" onClick={onClose} aria-label="Kapat" title="Kapat (Esc)">
                <X size={16} aria-hidden="true" />
              </button>
            </div>
          </header>
          {children}
        </>
      )}
    </dialog>
  )
}
