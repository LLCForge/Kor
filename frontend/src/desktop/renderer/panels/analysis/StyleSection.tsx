// Stil (§9 `kor style`, ADR 0047): kendi satırlarınızdan (git blame) ölçülen stil profili proje belleğine yazılır;
// ajan bu stile göre yazar. "Diff'i denetle" eklenen koddan profile uymayanları bulgu olarak gösterir.
import { ListChecks, PenLine } from 'lucide-react'
import { useId, useState } from 'react'
import type { AnalysisReport } from '../../../ipc-analysis.ts'
import type { StyleProfile } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { metricsApi } from './metrics-bridge.ts'
import { num } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

function StyleSection() {
  const id = useId()
  const [profile, setProfile] = useState<{ profile: StyleProfile; saved: boolean } | null>(null)
  const [check, setCheck] = useState<AnalysisReport | null>(null)
  const [staged, setStaged] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const run = async (what: 'style' | 'check') => {
    setBusy(what)
    setError(null)
    try {
      if (what === 'style') setProfile(await metricsApi.style())
      else setCheck((await metricsApi.styleCheck(staged)).report)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const p = profile?.profile
  return (
    <div className="km">
      <div className="ks-actions">
        <button type="button" className="btn small primary" disabled={busy !== null} onClick={() => void run('style')} data-kor-cmd="metrics.style">
          <PenLine size={14} aria-hidden="true" /> {busy === 'style' ? 'Ölçülüyor…' : 'Stilimi ölç ve kaydet'}
        </button>
        <button type="button" className="btn small" disabled={busy !== null} onClick={() => void run('check')} data-kor-cmd="metrics.styleCheck">
          <ListChecks size={14} aria-hidden="true" /> {busy === 'check' ? 'Denetleniyor…' : 'Diff’i denetle'}
        </button>
        <label className="ks-label" htmlFor={`${id}-st`}>
          <input id={`${id}-st`} type="checkbox" checked={staged} onChange={(e) => setStaged(e.target.checked)} /> Yalnızca sahnelenenler
        </label>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {p && profile && (
        <>
          <p role="status">
            {profile.saved
              ? 'Profil proje belleğine yazıldı (patterns.md, “style” bölümü); ajan bu stilde yazar.'
              : 'patterns.md içindeki “style” bölümünü düzenlemişsiniz, korundu; style.json güncellendi.'}
          </p>
          <p className="km-src" translate="no">{p.source}</p>
          <table className="km-table">
            <thead>
              <tr><th scope="col">Dil</th><th scope="col">Ad türü</th><th scope="col">Kural</th><th scope="col">Pay</th></tr>
            </thead>
            <tbody>
              {p.naming.map((c) => (
                <tr key={`${c.lang}:${c.category}`}>
                  <td translate="no">{c.lang}</td>
                  <td translate="no">{c.category}</td>
                  <td translate="no">{c.dominant}</td>
                  <td>{`%${num(c.share * 100)} / ${c.samples}`}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p>{`Fonksiyon: ${p.functions}; erken dönüş %${num(p.early_return_share * 100)}; ortalama ${num(p.avg_function_lines)} satır, karmaşıklık ${num(p.avg_complexity)}`}</p>
          <p>{`Yorum: kod satırı başına ${p.comment_density.toFixed(2)}; public fonksiyonların %${num(p.doc_share * 100)}’i belgelenmiş`}</p>
          {Object.keys(p.errors).length > 0 && (
            <p translate="no">{Object.entries(p.errors).map(([k, v]) => `${k} ×${v}`).join(', ')}</p>
          )}
          {p.preferences.length > 0 && (
            <ul className="km-list" translate="no">
              {p.preferences.map((x) => <li key={x}>{x}</li>)}
            </ul>
          )}
        </>
      )}
      {check && (
        check.findings.length === 0 ? (
          <p className="muted" role="status">Eklenen kodda stil profilinize uymayan bir şey yok.</p>
        ) : (
          <ul className="km-list" role="status">
            {check.findings.map((f) => (
              <li key={`${f.rule}:${f.file}:${f.line}`} className="km-row">
                <button type="button" className="km-loc" onClick={() => g3Actions.current?.openFile(f.file, f.line)} translate="no">{`${f.file}:${f.line}`}</button>
                <span>{f.message}</span>
                <span className="km-src" translate="no">{f.rule}</span>
              </li>
            ))}
          </ul>
        )
      )}
    </div>
  )
}

registerSection({ id: 'style', label: 'Stil', component: StyleSection })
