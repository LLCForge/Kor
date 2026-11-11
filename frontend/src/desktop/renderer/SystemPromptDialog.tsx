import { useEffect, useId, useRef, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from './api.ts'
import { uiLocale } from './ui-locale.ts'

interface EditorFormProps {
  titleId: string
  title: string
  description: string
  initial: string
  maxChars: number
  placeholder?: string
  resetLabel: string
  resetValue: string
  onSave: (text: string) => Promise<void>
  onClose: () => void
}

function EditorForm(props: EditorFormProps) {
  const [text, setText] = useState(props.initial)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setSaving(true)
    setError(null)
    try {
      await props.onSave(text)
      props.onClose()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <form className="dialog-body" onSubmit={(e) => void submit(e)}>
      <h2 id={props.titleId} className="dialog-title">{props.title}</h2>
      <p className="dialog-desc text-pretty">{props.description}</p>
      <label htmlFor={`${props.titleId}-text`} className="sr-only">{props.title}</label>
      <textarea
        id={`${props.titleId}-text`}
        className="dialog-textarea"
        value={text}
        placeholder={props.placeholder}
        onChange={(e) => setText(e.target.value)}
        spellCheck={false}
      />
      <p className="dialog-meta">
        {text.length.toLocaleString(uiLocale())} / {props.maxChars.toLocaleString(uiLocale())} karakter
      </p>
      {error && <p className="error-text" role="alert">{error}</p>}
      <div className="dialog-actions">
        <button type="button" className="btn ghost" onClick={() => setText(props.resetValue)} disabled={saving}>
          {props.resetLabel}
        </button>
        <div className="dialog-actions-right">
          <button type="button" className="btn" onClick={props.onClose} disabled={saving}>İptal</button>
          <button type="submit" className="btn primary" disabled={saving || text.length > props.maxChars}>
            {saving ? 'Kaydediliyor...' : 'Kaydet'}
          </button>
        </div>
      </div>
    </form>
  )
}

/** Yerel <dialog>: odak hapsi ve Esc ile kapanma tarayıcıdan gelir. */
function ModalShell({ open, onClose, labelledBy, children }: { open: boolean; onClose: () => void; labelledBy: string; children: React.ReactNode }) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) return
    if (open && !dialog.open) dialog.showModal()
    if (!open && dialog.open) dialog.close()
  }, [open])
  return (
    <dialog ref={dialogRef} className="dialog" onClose={onClose} aria-labelledby={labelledBy}>
      {children}
    </dialog>
  )
}

export function SystemPromptDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const titleId = useId()
  const { data, error, mutate } = useSWR(open ? 'systemPrompt' : null, () => api.getSystemPrompt())

  return (
    <ModalShell open={open} onClose={onClose} labelledBy={titleId}>
      {error && <p className="error-text dialog-body" role="alert">{errorMessage(error)}</p>}
      {open && !data && !error && <p className="dialog-body muted">Yükleniyor...</p>}
      {open && data && (
        <EditorForm
          titleId={titleId}
          title="Sistem istemi"
          description="Buraya yazdığın metin varsayılan talimatların yerine geçer ve hem masaüstünde hem CLI'da kullanılır. Çalışma dizini, platform, tarih, AGENTS.md ve hafıza her zaman otomatik eklenir. Açık sohbete de hemen uygulanır."
          initial={data.custom ?? data.defaultInstructions}
          maxChars={100_000}
          resetLabel="Varsayılana dön"
          resetValue={data.defaultInstructions}
          onSave={async (text) => {
            // Varsayılanla aynı metin kaydedilmez; varsayılan güncellenirse eski kopyada kalınmasın.
            await api.setSystemPrompt(text.trim() === data.defaultInstructions.trim() ? '' : text)
            await mutate()
          }}
          onClose={onClose}
        />
      )}
    </ModalShell>
  )
}

export function MemoryDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const titleId = useId()
  const { data, error, mutate } = useSWR(open ? 'memory' : null, () => api.getMemory())

  return (
    <ModalShell open={open} onClose={onClose} labelledBy={titleId}>
      {error && <p className="error-text dialog-body" role="alert">{errorMessage(error)}</p>}
      {open && data === undefined && !error && <p className="dialog-body muted">Yükleniyor...</p>}
      {open && data !== undefined && (
        <EditorForm
          titleId={titleId}
          title="Hafıza"
          description="Ajanın tüm sohbetlerde ve projelerde hatırladığı notlar. Ajan, sen istediğinde remember aracıyla buraya not ekler (her seferinde onayını ister). Buradan düzenleyebilir veya silebilirsin. Şifre ve anahtar yazma."
          initial={data}
          maxChars={20_000}
          placeholder={'- Paket yöneticisi olarak pnpm kullanıyorum\n- Commit mesajları İngilizce olsun'}
          resetLabel="Tümünü temizle"
          resetValue=""
          onSave={async (text) => {
            await api.setMemory(text)
            await mutate()
          }}
          onClose={onClose}
        />
      )}
    </ModalShell>
  )
}
