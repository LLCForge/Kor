// Kod yorumları (§12.2 `kor comment --lang`): bir dosyanın yorumlarını hedef dile çevirir, isteğe bağlı olarak
// yorumsuz fonksiyonlara tek satırlık yorum yazar. Önce fark gösterilir; dosyaya yalnızca "Uygula" ile yazılır.
import { Languages } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import { errorMessage } from '../../api.ts'
import { metricsApi } from './metrics-bridge.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

interface Result { diff: string; translated: number; added: number; kept: number; written: boolean }

function CommentSection() {
  const id = useId()
  const [file, setFile] = useState('')
  const [lang, setLang] = useState('tr')
  const [missing, setMissing] = useState(false)
  const [result, setResult] = useState<Result | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async (write: boolean) => {
    setBusy(true)
    setError(null)
    try {
      setResult(await metricsApi.comment(file.trim(), lang.trim(), missing, false, write))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const submit = (e: FormEvent) => {
    e.preventDefault()
    void run(false)
  }

  return (
    <div className="km">
      <form className="km-row" onSubmit={submit}>
        <label className="km-field" htmlFor={`${id}-f`}>
          <span>Dosya</span>
          <input id={`${id}-f`} value={file} onChange={(e) => { setFile(e.target.value); setResult(null) }} maxLength={500} spellCheck={false} translate="no" placeholder="src/main.rs" />
        </label>
        <label className="km-field" htmlFor={`${id}-l`}>
          <span>Hedef dil</span>
          <input id={`${id}-l`} value={lang} onChange={(e) => { setLang(e.target.value); setResult(null) }} maxLength={40} spellCheck={false} translate="no" placeholder="tr" />
        </label>
        <label className="ks-label" htmlFor={`${id}-m`}>
          <input id={`${id}-m`} type="checkbox" checked={missing} onChange={(e) => { setMissing(e.target.checked); setResult(null) }} /> Yorumsuz fonksiyonlara yorum yaz
        </label>
        <button type="submit" className="btn small primary" disabled={busy || !file.trim() || !lang.trim()} data-kor-cmd="metrics.comment">
          <Languages size={14} aria-hidden="true" /> {busy ? 'Çevriliyor…' : 'Önizle'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {result && (
        <div role="status">
          <p>{result.written ? `Yazıldı: ${result.translated} yorum çevrildi, ${result.added} yorum eklendi.` : `${result.translated} yorum çevrilecek, ${result.added} yorum eklenecek.`}</p>
          {result.kept > 0 && <p className="muted">{`${result.kept} yanıt denetimden geçmedi; o yorumlar olduğu gibi kaldı.`}</p>}
          {result.diff ? <pre className="km-prose" translate="no">{result.diff}</pre> : <p className="muted">Değişiklik yok.</p>}
          {result.diff && !result.written && (
            <button type="button" className="btn small" disabled={busy} onClick={() => void run(true)} data-kor-cmd="metrics.comment">
              Dosyaya uygula
            </button>
          )}
        </div>
      )}
    </div>
  )
}

registerSection({ id: 'comment', label: 'Kod yorumları', component: CommentSection })
