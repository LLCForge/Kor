// İstatistik (§9 `kor stats`, §9.1, ADR 0047): yalnızca kullanıcının gördüğü haftalık sayılar; takım sıralaması
// yalnızca `[stats] team = true` ile gelir.
import { BarChart3 } from 'lucide-react'
import { useId, useState } from 'react'
import type { DevStats } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { metricsApi } from './metrics-bridge.ts'
import { day, num } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

function StatsSection() {
  const id = useId()
  const [weeks, setWeeks] = useState(4)
  const [stats, setStats] = useState<DevStats | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      setStats(await metricsApi.stats(weeks))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="km">
      <p className="ks-note muted">Bu sayıları yalnızca siz görürsünüz; hiçbir şey bu bilgisayardan çıkmaz. Ajan satırları Kor görev kayıtlarından ve denetim kaydından gelir.</p>
      <div className="km-row">
        <label className="km-field" htmlFor={`${id}-w`}>
          <span>Dönem</span>
          <select id={`${id}-w`} value={weeks} onChange={(e) => setWeeks(Number(e.target.value))}>
            <option value={1}>Bu hafta</option>
            <option value={4}>Son 4 hafta</option>
            <option value={12}>Son 12 hafta</option>
          </select>
        </label>
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void run()} data-kor-cmd="metrics.stats">
          <BarChart3 size={14} aria-hidden="true" /> {busy ? 'Okunuyor…' : 'İstatistikleri göster'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {stats && (
        <>
          <p role="status" translate="no">{stats.author}</p>
          <p>{`Seri: ${stats.streak} gün (en uzun: ${stats.longest_streak})`}</p>
          <table className="km-table">
            <thead>
              <tr>
                <th scope="col">Hafta</th>
                <th scope="col">Commit</th>
                <th scope="col">Siz</th>
                <th scope="col">Ajan dosyalarında</th>
                <th scope="col">Ajan</th>
                <th scope="col">Düzeltme / özellik</th>
                <th scope="col">Kapsam</th>
              </tr>
            </thead>
            <tbody>
              {stats.weeks.map((w) => (
                <tr key={w.start}>
                  <td>{day(w.start)}</td>
                  <td>{`${w.commits} (${w.active_days} gün)`}</td>
                  <td>{`+${w.human.added} / −${w.human.deleted}`}</td>
                  <td>{`+${w.assisted.added} / −${w.assisted.deleted}`}</td>
                  <td title={`${w.agent_writes} ajan dosya yazması (denetim kaydı)`}>{`+${w.agent.added} / −${w.agent.deleted} · ${w.agent_commits} commit`}</td>
                  <td>{`${w.fixes} / ${w.features}`}</td>
                  <td>{w.coverage ? `%${num(w.coverage[0])} → %${num(w.coverage[1])}` : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {stats.team ? (
            <>
              <h4 className="ks-title">Takım sıralaması</h4>
              <ol className="km-list">
                {stats.team.map((r) => (
                  <li key={r.author}>
                    <span translate="no">{r.author}</span>
                    {`: ${r.commits} commit, +${r.added} / −${r.deleted}, ${r.fixes} düzeltme, ${r.features} özellik`}
                  </li>
                ))}
              </ol>
            </>
          ) : (
            <p className="muted">Takım sıralaması kapalı ([stats] team = true ile açılır).</p>
          )}
          <p className="km-src" translate="no">{stats.sources.join(' · ')}</p>
        </>
      )}
    </div>
  )
}

registerSection({ id: 'stats', label: 'İstatistik', component: StatsSection })
