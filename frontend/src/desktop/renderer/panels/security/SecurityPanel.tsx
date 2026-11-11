// Güvenlik görünümü (§12.3): sandbox durumu ve modu (§6.4), gizlilik modu (§6.5), sır kasası (§6.6),
// denetim kaydı (§6.7). Oturum araç izinleri kenar çubuğundaki Güvenlik düğmesinde kalır (g12).
import { Lock } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { AuditCard } from './AuditCard.tsx'
import { SandboxCard } from './SandboxCard.tsx'
import { VaultCard } from './VaultCard.tsx'

function PrivacyCard() {
  const id = useId()
  const { data, error, mutate } = useSWR('security:privacy', () => api.core.privacy())
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const toggle = async (enabled: boolean) => {
    setBusy(true)
    setActionError(null)
    try {
      await api.core.setPrivacy(enabled)
      await mutate({ enabled }, { revalidate: true })
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className={`kg-card${data?.enabled ? ' ks-privacy-on' : ''}`} aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Lock size={16} aria-hidden="true" />
        Gizlilik modu
        {data && <span className={`ks-badge${data.enabled ? ' on' : ''}`}>{data.enabled ? 'Açık' : 'Kapalı'}</span>}
      </h3>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="core.privacy">Okunuyor…</p>
      ) : (
        <label className="ks-check">
          <input type="checkbox" checked={data.enabled} disabled={busy} onChange={(e) => void toggle(e.target.checked)} data-kor-cmd="core.setPrivacy" />
          Yalnızca yerel sağlayıcılar; katalog güncellemesi ve tüm dış ağ çağrıları kapalı
        </label>
      )}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}

export function SecurityPanel(_: KorPanelProps) {
  return (
    <div className="ks">
      <SandboxCard />
      <PrivacyCard />
      <VaultCard />
      <AuditCard />
    </div>
  )
}
