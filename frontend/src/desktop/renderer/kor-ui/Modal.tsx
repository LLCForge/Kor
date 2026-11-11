import { useId, type ReactNode } from 'react'
import { Modal as NativeModal } from '../NameDialog.tsx'

export interface ModalProps {
  title: string
  description?: string
  onClose: () => void
  /** Alt eylem satırı (İptal / Kaydet) */
  footer?: ReactNode
  children?: ReactNode
  /** true: çıkış animasyonu oynar, ardından onClose gelir (Esc de aynı yoldan kapanır). */
  closing?: boolean
}

/**
 * Kor modalı: mevcut yerel `<dialog>` sarmalayıcısının (NameDialog.Modal; odak hapsi + Esc tarayıcıdan) üstünde
 * başlık, açıklama ve eylem satırı düzeni. Yeni diyaloglar bunu kullanır (§23.5).
 */
export function Modal({ title, description, onClose, footer, children, closing }: ModalProps) {
  const titleId = useId()
  return (
    <NativeModal onClose={onClose} labelledBy={titleId} closing={closing}>
      <div className="kor-modal">
        <h2 id={titleId} className="kor-modal-title">
          {title}
        </h2>
        {description && <p className="kor-modal-desc">{description}</p>}
        {children}
        {footer && <footer className="kor-modal-footer">{footer}</footer>}
      </div>
    </NativeModal>
  )
}
