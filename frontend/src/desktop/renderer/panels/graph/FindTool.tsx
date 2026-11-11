// "Kodda bul" (`kor find`, §8.9): doğal dille soru → dosya:satır yerleri, graf bağlantıları ve isteğe bağlı model açıklaması.
import { Search } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import type { FindApi, FindHit, FindResult, FindSource } from '../../../ipc-find.ts'
import { api, errorMessage } from '../../api.ts'
import { KIND_LABEL } from './model.ts'

const findApi = (api as unknown as { find: FindApi }).find

const SOURCE_LABEL: Record<FindSource, string> = { graph: 'graf', semantic: 'anlamsal', keyword: 'anahtar kelime' }

function hitLabel(h: FindHit): string {
  return `${h.file}:${h.line}${h.name ? ` ${h.name}` : ''}`
}

export function FindTool({ onSelect }: { onSelect: (id: string) => void }) {
  const [question, setQuestion] = useState('')
  const [explain, setExplain] = useState(true)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<FindResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const q = question.trim()
    if (q.length < 2) return
    setBusy(true)
    setError(null)
    try {
      setResult(await findApi.run(q, 10, explain))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const found = result?.found
  return (
    <div className="kg-tool">
      <form className="kg-row" onSubmit={(e) => void submit(e)}>
        <label className="kg-field">
          <span>Soru</span>
          <input
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            placeholder="Kullanıcı giriş yapınca ne oluyor?"
            maxLength={1000}
          />
        </label>
        <label className="kg-field">
          <span>
            <input type="checkbox" checked={explain} onChange={(e) => setExplain(e.target.checked)} /> Modelle açıkla
          </span>
        </label>
        <button type="submit" className="btn small" disabled={busy || question.trim().length < 2} data-kor-cmd="find.run">
          <Search size={14} aria-hidden="true" />
          {busy ? 'Aranıyor…' : 'Kodda bul'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {found && (
        <div className="kg-result" role="status">
          <span className="muted">
            {`Graf (${found.graph_nodes} düğüm) ve ${found.text_index === 'semantic' ? 'yerel anlamsal dizin' : 'anahtar kelime taraması'} kullanıldı`}
          </span>
          {found.notes.map((n) => (
            <span key={n} className="muted" translate="no">{n}</span>
          ))}
          {found.hits.length === 0 ? (
            <p className="muted">İlgili bir yer bulunamadı. Başka kelimeler deneyin.</p>
          ) : (
            <div className="kg-links">
              <h4>{`Yerler (${found.hits.length})`}</h4>
              <ul>
                {found.hits.map((h) => {
                  const meta = `${h.kind ? `${KIND_LABEL[h.kind]} · ` : ''}${h.sources.map((s) => SOURCE_LABEL[s]).join(', ')}`
                  const node = h.node
                  return (
                    <li key={`${h.file}:${h.line}`}>
                      {node ? (
                        <button type="button" className="kg-link" onClick={() => onSelect(node)} title={hitLabel(h)}>
                          <span translate="no">{hitLabel(h)}</span>
                          <span className="kg-link-meta">{meta}</span>
                        </button>
                      ) : (
                        <span className="kg-link" title={hitLabel(h)}>
                          <span translate="no">{hitLabel(h)}</span>
                          <span className="kg-link-meta">{meta}</span>
                        </span>
                      )}
                    </li>
                  )
                })}
              </ul>
            </div>
          )}
          {found.links.length > 0 && (
            <div className="kg-links">
              <h4>Akış (proje grafı)</h4>
              <ul translate="no">
                {found.links.map((l) => (
                  <li key={`${l.from}-${l.kind}-${l.to}`} className="kg-link">
                    <span>{`${l.from} → ${l.to}`}</span>
                    <span className="kg-link-meta">{l.kind.replace('_', ' ')}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {result.explanation && (
            <div className="kg-links">
              <h4>{result.provider ? `Açıklama (${result.provider})` : 'Açıklama'}</h4>
              <p translate="no">{result.explanation}</p>
            </div>
          )}
          {result.explainError && <p className="muted">{`Açıklama yok: ${result.explainError}`}</p>}
        </div>
      )}
    </div>
  )
}
