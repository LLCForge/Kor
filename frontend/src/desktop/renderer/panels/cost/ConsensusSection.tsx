// Consensus modu (§5.1 `kor consensus`): aynı soru 2–3 modele paralel gider; çoğunluğun uyuştuğu satırlar
// alınır, ayrışanlar model model gösterilir. Gölge modunun uyum geçmişi de burada (`[routing] shadow`).
import { Scale } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { percentText } from '../../ui-locale.ts'
import { costApi } from './api.ts'

type Result = Awaited<ReturnType<typeof costApi.consensus>>

export function ConsensusSection() {
  const id = useId()
  const [prompt, setPrompt] = useState('')
  const [models, setModels] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<Result | null>(null)
  const shadow = useSWR(['cost:shadow'], () => costApi.shadow(), { revalidateOnFocus: false })

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const list = models.split(',').map((m) => m.trim()).filter(Boolean)
      setResult(await costApi.consensus(prompt.trim(), list.length > 0 ? list : null))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="g14-section" aria-labelledby={`${id}-h`}>
      <h3 id={`${id}-h`} className="g14-h">Consensus</h3>
      <p className="g7-note text-pretty">Aynı soru 2–3 ucuz modele paralel gider. Çoğunluğun verdiği satırlar alınır; ayrışan kısımlar model model gösterilir, hangisini tutacağınızı siz seçersiniz.</p>
      <form className="g2c-form" onSubmit={(e) => void submit(e)}>
        <label htmlFor={`${id}-p`}>Soru</label>
        <textarea id={`${id}-p`} className="text-input" rows={3} value={prompt} onChange={(e) => setPrompt(e.target.value)} maxLength={20000} />
        <label htmlFor={`${id}-m`}>Modeller (boş: ayarlardaki)</label>
        <input id={`${id}-m`} className="text-input" value={models} onChange={(e) => setModels(e.target.value)} placeholder="sağlayıcı/model, sağlayıcı/model" spellCheck={false} maxLength={600} translate="no" />
        <span />
        <button type="submit" className="btn small primary" disabled={busy || !prompt.trim()} data-kor-cmd="cost.consensus">
          <Scale size={14} aria-hidden="true" /> {busy ? 'Soruluyor…' : 'Modellere sor'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {result && (
        <div role="status" className="g2c-stack">
          {result.failed.map(([m, e]) => <p key={m} className="error-text" translate="no">{`${m}: ${e}`}</p>)}
          <p>{`${result.answers.length} model, satırların ${percentText(result.agreement)} kadarında uyuştu.`}</p>
          {result.agreed.length > 0 ? (
            <pre className="km-prose" translate="no">{result.agreed.join('\n')}</pre>
          ) : (
            <p className="muted">Çoğunluğun uyuştuğu satır yok.</p>
          )}
          {result.disputed.map((v, i) => (
            <div key={i}>
              <p className="g7-note">Ayrışan: <span className="g7-mono" translate="no">{v.models.join(', ')}</span></p>
              <pre className="km-prose" translate="no">{v.lines.join('\n')}</pre>
            </div>
          ))}
        </div>
      )}
      <h4 className="g14-h">Gölge modu</h4>
      {shadow.data && shadow.data.length > 0 ? (
        <ul className="km-list" data-kor-cmd="cost.shadow">
          {shadow.data.map((r) => (
            <li key={`${r.main}>${r.shadow}`} translate="no">{`${r.main} → ${r.shadow}: ${percentText(r.agreement)} (${r.samples})`}</li>
          ))}
        </ul>
      ) : (
        <p className="muted" data-kor-cmd="cost.shadow">Gölge modu henüz bir görev karşılaştırmadı. Ayarlar'da bir gölge model seçince her görevden sonra arka planda ölçülür.</p>
      )}
    </section>
  )
}
