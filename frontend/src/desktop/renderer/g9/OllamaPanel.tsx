// Yerel Ollama modelleri: liste, bellekteki modeller (VRAM), indirme ilerlemesi, boşaltma. Yalnızca kullanıcı tıklayınca istek gider.
import { CircleStop, Download, HardDrive, RefreshCw } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { api, errorMessage } from '../api.ts'
import { CATALOG_KEY } from '../g7-bus.ts'
import { uiLocale } from '../ui-locale.ts'

const GB = 1024 ** 3
const size = (bytes: number) => `${(bytes / GB).toLocaleString(uiLocale(), { maximumFractionDigits: 1 })} GB`

export function OllamaPanel() {
  const id = useId()
  const { mutate: globalMutate } = useSWRConfig()
  const [polling, setPolling] = useState(false)
  const { data, error, mutate, isLoading } = useSWR('g9:ollama', () => api.g9.ollamaList(), {
    revalidateOnFocus: false,
    refreshInterval: polling ? 1_000 : 0,
    onSuccess: (view) => setPolling(view.pulls.some((p) => !p.done)),
  })
  const [name, setName] = useState('')
  const [message, setMessage] = useState<string | null>(null)

  const act = async (fn: () => Promise<unknown>) => {
    setMessage(null)
    try {
      await fn()
      await mutate()
      await globalMutate(CATALOG_KEY)
    } catch (err) {
      setMessage(errorMessage(err))
    }
  }

  const pull = (e: FormEvent) => {
    e.preventDefault()
    void act(async () => {
      await api.g9.ollamaPull(name.trim())
      setName('')
      setPolling(true)
    })
  }

  return (
    <div className="g9-ollama">
      <div className="g7-inline">
        <HardDrive size={14} aria-hidden="true" />
        <span className="g7-setting-title">Yerel modeller</span>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Yerel model listesini yenile" title="Yenile">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {isLoading && <p className="muted">Ollama'ya bağlanılıyor...</p>}
      {error && <p className="error-text">{errorMessage(error)} (Ollama çalışıyor mu? Varsayılan adres http://localhost:11434)</p>}
      {data && (
        <ul className="g9-profiles" aria-label="Yüklü Ollama modelleri">
          {data.models.length === 0 && <li className="muted">Yüklü model yok.</li>}
          {data.models.map((m) => {
            const loaded = data.running.find((r) => r.name === m.name)
            return (
              <li key={m.name} className="g9-profile">
                <span className="g9-profile-name g7-mono">{m.name}</span>
                <span className="g7-note">
                  {size(m.sizeBytes)}
                  {m.parameterSize ? ` · ${m.parameterSize}` : ''}
                  {m.quantization ? ` · ${m.quantization}` : ''}
                </span>
                {loaded && <span className="badge g9-on" title="Bellekte">{`Bellekte · VRAM ${size(loaded.vramBytes)}`}</span>}
                <span className="g9-grow" />
                {loaded && (
                  <button type="button" className="btn small" onClick={() => void act(() => api.g9.ollamaUnload(m.name))}>
                    Bellekten boşalt
                  </button>
                )}
              </li>
            )
          })}
          {data.pulls.map((p) => (
            <li key={`pull-${p.name}`} className="g9-profile">
              <span className="g9-profile-name g7-mono">{p.name}</span>
              <span className={p.error ? 'error-text' : 'g7-note'} role="status">
                {p.error ?? (p.total && p.completed !== null ? `${p.status} · %${Math.floor((p.completed / p.total) * 100)}` : p.status)}
              </span>
              <span className="g9-grow" />
              {!p.done && (
                <button type="button" className="action-btn" onClick={() => void act(() => api.g9.ollamaCancelPull(p.name))} aria-label={`${p.name} indirmesini durdur`} title="Durdur">
                  <CircleStop size={14} aria-hidden="true" />
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <form className="g9-add-key" onSubmit={pull}>
        <label htmlFor={`${id}-pull`} className="sr-only">İndirilecek model adı</label>
        <input id={`${id}-pull`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="ör. qwen3:8b" spellCheck={false} />
        <button type="submit" className="btn small" disabled={!name.trim()}>
          <Download size={14} aria-hidden="true" /> İndir
        </button>
      </form>
      <p className="g7-note">İndirmeyi yerel Ollama sunucusu kendi kayıt deposundan yapar; uygulama yalnızca localhost ile konuşur.</p>
      {message && <p className="error-text" role="status">{message}</p>}
    </div>
  )
}
