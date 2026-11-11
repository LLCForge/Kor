// Genel komut çalıştırıcı (ADR 0031): paletin "Tüm komutlar" sayfasından, kendi paneli olmayan bir parity.toml
// köprü yöntemi seçildiğinde açılır. Bağımsız değişkenler JSON (konumsal dizi), sonuç kopyalanabilir metin.
import { useId, useState, type FormEvent } from 'react'
import { api, errorMessage } from './api.ts'
import { confirmAction } from './ConfirmDialog.tsx'
import { CopyButton } from './CopyButton.tsx'
import { Modal } from './NameDialog.tsx'
import type { ParityCommand } from './parity-commands.gen.ts'
import { CONFIRM_METHODS, formatResult, parseArgs } from './parity-palette.ts'
import './styles/parity.css'

type BridgeFn = (...args: unknown[]) => Promise<unknown>

/** `api` bir Proxy'dir: her `ns.method` yolu `kor_call`'a gider (kor-bridge.ts). */
function bridgeMethod(method: string): BridgeFn {
  const [ns, name] = method.split('.') as [string, string]
  return (api as unknown as Record<string, Record<string, BridgeFn>>)[ns]![name]!
}

type Outcome = { kind: 'ok'; text: string } | { kind: 'error'; text: string }

/** `initialArgs`: "/seo audit https://site" gibi slash komutundan kalan sözcükler; JSON dizisi olarak doldurulur. */
export function ParityRunner({ command, onClose, initialArgs }: { command: ParityCommand; onClose: () => void; initialArgs?: string }) {
  const titleId = useId()
  const argsId = useId()
  const [args, setArgs] = useState(() => JSON.stringify((initialArgs ?? '').split(/\s+/).filter(Boolean)))
  const [busy, setBusy] = useState(false)
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const parsed = parseArgs(args)

  const run = async (e: FormEvent) => {
    e.preventDefault()
    if (!parsed.ok || busy) return
    if (CONFIRM_METHODS.has(command.method)) {
      const ok = await confirmAction({
        title: command.name,
        message: `“${command.method}” sistemde değişiklik yapabilir ya da ücretli bir çağrı başlatabilir. Devam edilsin mi?`,
        confirmLabel: 'Çalıştır',
        danger: true,
      })
      if (!ok) return
    }
    setBusy(true)
    setOutcome(null)
    try {
      setOutcome({ kind: 'ok', text: formatResult(await bridgeMethod(command.method)(...parsed.args)) })
    } catch (err) {
      setOutcome({ kind: 'error', text: errorMessage(err) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <form className="dialog-body parity-runner" onSubmit={(e) => void run(e)} data-kor-cmd={command.method}>
        <h2 id={titleId} className="dialog-title">{command.name}</h2>
        <p className="parity-runner-meta">
          <code translate="no">{command.method}</code>
          {command.cli && <code translate="no">{command.cli}</code>}
        </p>
        <label htmlFor={argsId} className="dialog-desc">Bağımsız değişkenler (JSON dizi; boş = yok)</label>
        <textarea
          id={argsId}
          className="text-input dialog-textarea parity-runner-args"
          value={args}
          onChange={(e) => setArgs(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          translate="no"
          aria-invalid={!parsed.ok}
          aria-describedby={`${argsId}-err`}
        />
        <p id={`${argsId}-err`} className="error-text g4-field-error" role="alert">{parsed.ok ? '' : parsed.error}</p>
        {outcome && (
          <section aria-live="polite">
            <div className="parity-runner-head">
              <p className={outcome.kind === 'error' ? 'error-text' : 'dialog-desc'}>{outcome.kind === 'error' ? 'Hata' : 'Sonuç'}</p>
              <CopyButton getText={() => outcome.text} label="Sonucu kopyala" />
            </div>
            <pre className="parity-runner-out" tabIndex={0} translate="no">{outcome.text}</pre>
          </section>
        )}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose}>Kapat</button>
            <button type="submit" className={`btn ${CONFIRM_METHODS.has(command.method) ? 'danger' : 'primary'}`} disabled={busy || !parsed.ok}>
              {busy ? 'Çalışıyor...' : 'Çalıştır'}
            </button>
          </div>
        </div>
      </form>
    </Modal>
  )
}
