// Mesaj kutusundaki mikrofon düğmesi (§10, ADR 0048): yerel whisper.cpp ile 10 sn kaydı metne çevirir
// ve taslağa ekler. Ses makineden çıkmaz. Bileşenler kurulu değilse "Görsel girdi" paneline yönlendirir.
import { Loader2, Mic } from 'lucide-react'
import { useState } from 'react'
import { errorMessage } from '../../api.ts'
import { visualApi } from './api.ts'
import { Tooltip } from '../../kor-ui/index.ts'

interface VoiceButtonProps {
  draft: string
  onDraftChange: (text: string) => void
  disabled?: boolean
}

const SECONDS = 10

export function VoiceButton({ draft, onDraftChange, disabled }: VoiceButtonProps) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const listen = async () => {
    setError(null)
    setBusy(true)
    try {
      const status = await visualApi.voiceStatus()
      if (!status.ready) {
        setError('Yerel konuşma tanıma kurulu değil: Görsel girdi panelinden whisper.cpp bileşenlerini kur.')
        window.dispatchEvent(new CustomEvent('panel:view', { detail: 'visual' }))
        return
      }
      const { text } = await visualApi.voice(SECONDS)
      onDraftChange(draft.trim() ? `${draft.trimEnd()} ${text}` : text)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const label = busy ? `Dinleniyor (${SECONDS} sn)…` : error ?? `Sesle yaz (${SECONDS} sn, yerel whisper.cpp)`
  return (
    <Tooltip text={label}>
      <button
        type="button"
        className="action-btn"
        onClick={() => void listen()}
        disabled={disabled || busy}
        aria-label={label}
        aria-busy={busy}
        data-kor-cmd="visual.voice visual.voiceStatus"
      >
        {busy ? <Loader2 size={16} className="tool-icon spin" aria-hidden="true" /> : <Mic size={16} aria-hidden="true" />}
      </button>
    </Tooltip>
  )
}
