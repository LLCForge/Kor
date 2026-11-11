import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react'
import { validateFileName } from '../../core/file-names.ts'
import { errorMessage } from './api.ts'
import { motionMs } from './motion.ts'

/**
 * Yerel <dialog>: odak hapsi ve Esc ile kapanma tarayıcıdan gelir. Bileşen kaldırılınca öğe DOM'dan çıkar ve
 * tarayıcı diyaloğu kendisi kapatır; temizlikte close() çağrılmaz (StrictMode çift etkisi onClose'u tetiklerdi).
 * Esc ya da `closing` ile kapanışta önce çıkış animasyonu (`data-closing`, `--kor-dur-menu`) oynar, sonra close()
 * çağrılır: odak tarayıcı tarafından açan öğeye döner ve onClose bir kez gelir (§23.1).
 */
export function Modal({ onClose, labelledBy, children, closing = false }: { onClose: () => void; labelledBy: string; children: ReactNode; closing?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    if (dialog && !dialog.open) dialog.showModal()
  }, [])
  const animateClose = () => {
    const dialog = ref.current
    if (!dialog?.open || dialog.hasAttribute('data-closing')) return
    const ms = motionMs('menu')
    if (ms === 0) return dialog.close()
    dialog.setAttribute('data-closing', '')
    window.setTimeout(() => dialog.close(), ms)
  }
  useEffect(() => {
    if (closing) animateClose()
  }, [closing])
  return (
    <dialog
      ref={ref}
      className="dialog g4-dialog"
      onClose={onClose}
      onCancel={(e) => {
        e.preventDefault()
        animateClose()
      }}
      aria-labelledby={labelledBy}
    >
      {children}
    </dialog>
  )
}

interface NameDialogProps {
  title: string
  description: string
  initial: string
  submitLabel: string
  onSubmit: (name: string) => Promise<void>
  onClose: () => void
}

/** Yeni dosya/klasör ve yeniden adlandırma için ad sorar; ad hem burada hem ana süreçte doğrulanır. */
export function NameDialog({ title, description, initial, submitLabel, onSubmit, onClose }: NameDialogProps) {
  const titleId = useId()
  const inputId = useId()
  const [name, setName] = useState(initial)
  const [busy, setBusy] = useState(false)
  const [serverError, setServerError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const problem = name === '' ? null : validateFileName(name)

  useEffect(() => {
    const input = inputRef.current
    if (!input) return
    input.focus()
    // Yeniden adlandırmada uzantı seçilmez: genelde yalnızca ad değişir.
    const dot = initial.lastIndexOf('.')
    input.setSelectionRange(0, dot > 0 ? dot : initial.length)
  }, [initial])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (problem || name === '') return
    setBusy(true)
    setServerError(null)
    try {
      await onSubmit(name)
      onClose()
    } catch (err) {
      setServerError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <form className="dialog-body" onSubmit={(e) => void submit(e)}>
        <h2 id={titleId} className="dialog-title">{title}</h2>
        <p className="dialog-desc">{description}</p>
        <label htmlFor={inputId} className="sr-only">Ad</label>
        <input
          id={inputId}
          ref={inputRef}
          className="text-input"
          value={name}
          onChange={(e) => {
            setName(e.target.value)
            setServerError(null)
          }}
          spellCheck={false}
          autoComplete="off"
          aria-invalid={problem !== null || serverError !== null}
          aria-describedby={`${inputId}-err`}
          maxLength={255}
        />
        <p id={`${inputId}-err`} className="error-text g4-field-error" role="alert">
          {problem ?? serverError ?? ''}
        </p>
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose} disabled={busy}>İptal</button>
            <button type="submit" className="btn primary" disabled={busy || problem !== null || name === '' || name === initial}>
              {busy ? 'İşleniyor...' : submitLabel}
            </button>
          </div>
        </div>
      </form>
    </Modal>
  )
}
