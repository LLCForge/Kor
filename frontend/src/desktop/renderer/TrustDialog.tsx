import { ShieldAlert } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import type { TrustInfo } from '../g7-types.ts'
import { errorMessage } from './api.ts'
import { displayPath } from './display-path.ts'

interface TrustDialogProps {
  info: TrustInfo
  onDecide: (trusted: boolean) => Promise<void>
  /** Karar vermeden kapatma: klasör bu oturumda güvenilmeyen olarak kalır */
  onDismiss: () => void
}

/** Proje dosyaları olan bir klasör ilk kez açıldığında gösterilir. */
export function TrustDialog({ info, onDecide, onDismiss }: TrustDialogProps) {
  const titleId = useId()
  const ref = useRef<HTMLDialogElement>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const dialog = ref.current
    if (dialog && !dialog.open) dialog.showModal()
  }, [])

  const decide = async (trusted: boolean) => {
    setBusy(true)
    setError(null)
    try {
      await onDecide(trusted)
    } catch (err) {
      setError(errorMessage(err))
      setBusy(false)
    }
  }

  return (
    <dialog ref={ref} className="dialog g7-trust" aria-labelledby={titleId} onClose={onDismiss}>
      <div className="dialog-body">
        <h2 id={titleId} className="dialog-title g7-trust-title">
          <ShieldAlert size={22} aria-hidden="true" />
          Bu klasöre güveniyor musun?
        </h2>
        <p className="g7-mono g7-path">{displayPath(info.root)}</p>
        <p className="dialog-desc text-pretty">Bu klasörde ajanın davranışını değiştirebilecek proje dosyaları var:</p>
        <ul className="g7-bullets g7-mono">
          {info.projectFiles.map((f) => (
            <li key={f}>{f}</li>
          ))}
        </ul>
        <p className="dialog-desc text-pretty">
          <strong>Güvenirsen</strong> AGENTS.md talimatları ve .kor/agents altındaki ajan tanımları ajana yüklenir. <strong>Güvenmezsen</strong> bunlar
          yüklenmez ve bu klasördeki sohbetlerde izin modu Manuel'e sabitlenir (her değişiklik sorulur). Başkasından indirdiğin veya içeriğini bilmediğin
          depolara güvenme. Kararını sonra Ayarlar → İzinler'den değiştirebilirsin.
        </p>
        {error && <p className="error-text" role="alert">{error}</p>}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={() => void decide(false)} disabled={busy}>
              Güvenme
            </button>
            <button type="button" className="btn primary" onClick={() => void decide(true)} disabled={busy}>
              Güven
            </button>
          </div>
        </div>
      </div>
    </dialog>
  )
}
