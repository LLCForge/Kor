// Kod arkeolojisi (§9 `kor explain`, §12.2 `--level junior`, ADR 0047): git geçmişi, yazarlar (yalnızca bu
// bilgisayarda), ad değişiklikleri, refactor'lar, tehlikeli bölgeler, graf; isteğe bağlı model açıklaması
// (modele yazar adları gitmez).
import { BookOpenText } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import type { Archaeology, ExplainLevel } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { metricsApi } from './metrics-bridge.ts'
import { day, num } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

interface Result {
  facts: Archaeology
  explanation: string | null
  explanationError: string | null
}

function Facts({ a }: { a: Archaeology }) {
  return (
    <>
      <p translate="no">{`${a.file} (${a.lang ?? '—'}, ${a.stats?.lines ?? 0} satır, ${a.stats?.functions.length ?? 0} fonksiyon)`}</p>
      {a.tracked ? (
        <p>{`${a.commits} commit · ilk ${a.first ? day(a.first.time) : '—'} (${a.age_days ?? 0} gün önce) · son ${a.last ? day(a.last.time) : '—'} (${a.last_change_days ?? 0} gün önce) · 90 gün: ${a.churn_90d}, 365 gün: ${a.churn_365d}`}</p>
      ) : (
        <p className="muted">Git geçmişinde yok.</p>
      )}
      {a.authors.length > 0 && (
        <>
          <h4 className="ks-title">Yazarlar (yalnızca bu bilgisayarda)</h4>
          <ul className="km-list">
            {a.authors.map((x) => (
              <li key={x.email}>
                <span translate="no">{`${x.name} <${x.email}>`}</span>
                {`: satırların %${num(x.share)}’i, ${x.commits} commit`}
              </li>
            ))}
          </ul>
        </>
      )}
      {a.renames.length > 0 && (
        <ul className="km-list" translate="no">
          {a.renames.map((r) => <li key={r.commit.hash + r.to}>{`${r.from} → ${r.to} (${day(r.commit.time)}, ${r.commit.hash})`}</li>)}
        </ul>
      )}
      <p>{a.refactors.length === 0 ? 'Refactor: hiç yapılmamış.' : `Refactor commit’leri: ${a.refactors.length}`}</p>
      {a.danger.length === 0 ? (
        <p className="muted">Tehlikeli bölge yok.</p>
      ) : (
        <>
          <h4 className="ks-title">Tehlikeli bölgeler (karmaşıklık × satır sahibi commit)</h4>
          <ul className="km-list">
            {a.danger.map((z) => (
              <li key={z.line} className="km-row">
                <button type="button" className="km-loc" onClick={() => g3Actions.current?.openFile(a.file, z.line)} translate="no">{`${z.name} ${z.line}–${z.end}`}</button>
                <span>{`karmaşıklık ${z.complexity} × ${z.commits} commit = ${z.score}; son 90 günde ${z.recent_lines} satır`}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {a.graph && (
        <p>
          {`Graf: topluluk “${a.graph.community ?? '—'}”, ${a.graph.dependents_total} doğrudan bağımlı`}
          {a.graph.co_changes.length > 0 && <span className="km-src" translate="no">{` · birlikte değişir: ${a.graph.co_changes.map(([f, r]) => `${f} (%${Math.round(r * 100)})`).join(', ')}`}</span>}
        </p>
      )}
      <p className="km-src" translate="no">{a.sources.join(' · ')}</p>
    </>
  )
}

function ExplainSection() {
  const id = useId()
  const [file, setFile] = useState('')
  const [level, setLevel] = useState<ExplainLevel>('normal')
  const [offline, setOffline] = useState(false)
  const [result, setResult] = useState<Result | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      setResult(await metricsApi.explain(file.trim(), level, offline))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="km">
      <form className="km-row" onSubmit={(e) => void submit(e)}>
        <label className="km-field" htmlFor={`${id}-f`}>
          <span>Dosya</span>
          <input id={`${id}-f`} value={file} onChange={(e) => setFile(e.target.value)} maxLength={500} spellCheck={false} translate="no" placeholder="src/main.rs" />
        </label>
        <label className="km-field" htmlFor={`${id}-l`}>
          <span>Düzey</span>
          <select id={`${id}-l`} value={level} onChange={(e) => setLevel(e.target.value as ExplainLevel)}>
            <option value="junior">Yeni başlayan (sade dil)</option>
            <option value="normal">Normal</option>
            <option value="expert">Uzman</option>
          </select>
        </label>
        <label className="ks-label" htmlFor={`${id}-o`}>
          <input id={`${id}-o`} type="checkbox" checked={offline} onChange={(e) => setOffline(e.target.checked)} /> Yalnızca olgular (model yok)
        </label>
        <button type="submit" className="btn small primary" disabled={busy || !file.trim()} data-kor-cmd="metrics.explain">
          <BookOpenText size={14} aria-hidden="true" /> {busy ? 'İnceleniyor…' : 'Açıkla'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {result && (
        <div role="status">
          <Facts a={result.facts} />
          {result.explanation && <div className="km-prose" translate="no">{result.explanation}</div>}
          {result.explanationError && <p className="muted">{`Açıklama alınamadı: ${result.explanationError}`}</p>}
        </div>
      )}
    </div>
  )
}

registerSection({ id: 'explain', label: 'Kod arkeolojisi', component: ExplainSection })
