import { RotateCcw, ShieldAlert, ShieldCheck, WifiOff } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { isNetworkErrorMessage } from '../../core/usage-history.ts'
import { api, errorMessage } from './api.ts'
import type { ChatItem } from './chat-state.ts'
import { closeG7Dialog, trustKey, useG7Dialog } from './g7-bus.ts'
import { ModelCompare } from './ModelCompare.tsx'
import { ModelProbe } from './ModelProbe.tsx'
import { SettingsDialog } from './SettingsDialog.tsx'
import { TrustDialog } from './TrustDialog.tsx'

interface G7LayerProps {
  root: string | null
  currentModel: string
  items: readonly ChatItem[]
  running: boolean
  onRetry: () => void
  onModelChange: (id: string) => void
  onOpenMemory: () => void
  onOpenSystemPrompt: () => void
  onSessionsCleared: () => void
}

/** navigator.onLine + ana süreçten Venice erişilebilirlik denetimi; çevrimdışıyken 10 sn'de bir yeniden dener. */
function useOnline(suspect: boolean): boolean {
  const { data, mutate } = useSWR('g7:online', async () => navigator.onLine && (await api.g7CheckOnline()), {
    refreshInterval: (latest) => (latest === false ? 10_000 : 0),
    revalidateOnFocus: true,
  })
  useEffect(() => {
    const recheck = () => void mutate()
    window.addEventListener('online', recheck)
    window.addEventListener('offline', recheck)
    return () => {
      window.removeEventListener('online', recheck)
      window.removeEventListener('offline', recheck)
    }
  }, [mutate])
  // Ağ hatasıyla biten bir tur, bağlantının koptuğunun ilk işareti olabilir.
  useEffect(() => {
    if (suspect) void mutate()
  }, [suspect, mutate])
  return data !== false
}

export function G7Layer(props: G7LayerProps) {
  const dialog = useG7Dialog()
  const last = props.items.at(-1)
  const networkFailure = !props.running && last?.kind === 'error' && isNetworkErrorMessage(last.text)
  const online = useOnline(networkFailure)
  const trust = useSWR(props.root ? trustKey(props.root) : null, () => api.g7GetTrust())
  const [dismissedTrust, setDismissedTrust] = useState<string | null>(null)

  const info = trust.data
  const askTrust = info !== undefined && info !== null && info.decision === null && info.projectFiles.length > 0 && dismissedTrust !== info.root
  const untrusted = info !== undefined && info !== null && !info.trusted && !askTrust

  const decideTrust = async (trusted: boolean) => {
    await api.g7SetTrust(trusted)
    await trust.mutate()
  }
  const [trusting, setTrusting] = useState(false)
  const [trustError, setTrustError] = useState<string | null>(null)
  const trustNow = async () => {
    setTrusting(true)
    setTrustError(null)
    try {
      await decideTrust(true)
    } catch (err: unknown) {
      setTrustError(`Güven kaydedilemedi: ${errorMessage(err)}`)
    } finally {
      setTrusting(false)
    }
  }

  return (
    <>
      <div className="g7-bands" aria-live="polite">
        {!online && (
          <div className="g7-band offline pop-in" role="status">
            <WifiOff size={14} aria-hidden="true" />
            <span>Çevrimdışı: sohbet API'sine ulaşılamıyor. Bağlantı gelince bu uyarı kendiliğinden kalkar.</span>
          </div>
        )}
        {networkFailure && (
          <div className="g7-band pop-in" role="status">
            <span>Bağlantı hatası yüzünden son mesaj tamamlanamadı.</span>
            <button type="button" className="btn small" onClick={props.onRetry} disabled={!online}>
              <RotateCcw size={14} aria-hidden="true" /> Yeniden dene
            </button>
          </div>
        )}
        {untrusted && (
          <div className="g7-band warn pop-in" role="status">
            <ShieldAlert size={14} aria-hidden="true" />
            <span>Güvenilmeyen klasör: AGENTS.md ve proje ajanları yüklenmedi, izin modu Manuel'e sabit.</span>
            <button type="button" className="btn small primary" disabled={trusting} onClick={() => void trustNow()} data-kor-cmd="g7SetTrust">
              <ShieldCheck size={14} aria-hidden="true" /> {trusting ? 'Kaydediliyor…' : 'Bu klasöre güven'}
            </button>
            {trustError && <span className="g7-band-error" role="alert">{trustError}</span>}
          </div>
        )}
      </div>

      {askTrust && <TrustDialog key={info.root} info={info} onDecide={decideTrust} onDismiss={() => setDismissedTrust(info.root)} />}

      {dialog?.kind === 'settings' && (
        <SettingsDialog
          open
          initialSection={dialog.section}
          onClose={closeG7Dialog}
          onOpenMemory={() => {
            closeG7Dialog()
            props.onOpenMemory()
          }}
          onOpenSystemPrompt={() => {
            closeG7Dialog()
            props.onOpenSystemPrompt()
          }}
          onSessionsCleared={props.onSessionsCleared}
          root={props.root}
        />
      )}
      {dialog?.kind === 'compare' && <ModelCompare open onClose={closeG7Dialog} currentModel={props.currentModel} onUseModel={props.onModelChange} />}
      {dialog?.kind === 'probe' && <ModelProbe open onClose={closeG7Dialog} currentModel={props.currentModel} initialModels={dialog.models} />}
    </>
  )
}
