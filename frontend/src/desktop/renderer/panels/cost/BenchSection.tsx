// `kor bench` masaüstünden (§5.1, §7.5 graf açık/kapalı). Çalıştırmak gerçek para harcar: her çalıştırma,
// tahmin ve --max-cost üst sınırı gösterilerek açıkça onaylanır. İlerleme `kor:bench/progress`, sonuç `kor:bench/done`.
import { Play, Square } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { BenchDone, BenchOptions, BenchProgress, BenchReport, BenchSummary } from '../../../ipc-cost.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { HBars } from '../../g14-charts.tsx'
import { formatMs, formatTokens } from '../../g7-format.ts'
import { benchApi } from './api.ts'
import { graphMetrics, money, parseModels } from './shape.ts'

type GraphMode = '' | 'both' | 'on' | 'off'

export function BenchSection() {
  const id = useId()
  const state = useSWR(['cost:bench'], () => benchApi.result(), { revalidateOnFocus: false })
  const [models, setModels] = useState('')
  const [graph, setGraph] = useState<GraphMode>('both')
  const [maxCost, setMaxCost] = useState('0.50')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [progress, setProgress] = useState<BenchProgress | null>(null)
  const [done, setDone] = useState<BenchDone | null>(null)

  const { mutate } = state
  useEffect(() => benchApi.onProgress(setProgress), [])
  useEffect(
    () =>
      benchApi.onDone((d) => {
        setDone(d)
        setProgress(null)
        void mutate()
      }),
    [mutate],
  )

  const running = state.data?.running === true
  const last = done ?? state.data?.last ?? null

  const run = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    const cap = Number(maxCost)
    if (!Number.isFinite(cap) || cap < 0) return setError('--max-cost sıfır ya da pozitif bir tutar olmalı.')
    const opts: BenchOptions = { maxCost: cap }
    const list = parseModels(models)
    if (list.length > 0) opts.models = list
    if (graph) opts.graph = graph
    setBusy(true)
    try {
      const est = await benchApi.estimate(opts)
      const unknown = est.unknown.length > 0 ? ` Fiyatı bilinmeyen modeller: ${est.unknown.join(', ')}; bunların maliyeti sınırlanamaz.` : ''
      const over = est.withinBudget ? '' : ' Tahmin üst sınırı aşıyor ya da sınırlanamıyor.'
      const ok = await confirmAction({
        title: 'Bench çalıştırılsın mı?',
        message: `${est.jobs} iş, tahmini ${money(est.usd, true, 4)}; --max-cost ${money(est.maxCost, false)}.${over}${unknown} Gerçek sağlayıcı çağrıları yapılır ve ücretlendirilir.`,
        confirmLabel: 'Çalıştır ve öde',
        danger: !est.withinBudget,
      })
      if (!ok) return
      setDone(null)
      await benchApi.start({ ...opts, confirm: true })
      await mutate()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const cancel = async () => {
    try {
      await benchApi.cancel()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  return (
    <div className="g2c-stack" data-kor-event="kor:bench/progress kor:bench/done" data-kor-cmd="bench.result">
      <form className="g14-section" onSubmit={(e) => void run(e)} aria-labelledby={`${id}-h`} data-kor-cmd="bench.start">
        <h3 id={`${id}-h`} className="g14-h">Model yarışı</h3>
        <p className="g7-note text-pretty">
          Görevler .kor/bench.toml'dan okunur ve projenin kopyasında çalışır (yalnızca güvenilen projede). Model listesi boşsa dosyadakiler kullanılır.
        </p>
        <div className="g2c-form">
          <label htmlFor={`${id}-models`}>Modeller</label>
          <input id={`${id}-models`} className="text-input" value={models} onChange={(e) => setModels(e.target.value)} placeholder="bench.toml'daki" spellCheck={false} translate="no" />
          <label htmlFor={`${id}-graph`}>Proje grafı</label>
          <select id={`${id}-graph`} className="g7-select" value={graph} onChange={(e) => setGraph(e.target.value as GraphMode)}>
            <option value="both">Açık ve kapalı karşılaştır</option>
            <option value="on">Yalnız açık</option>
            <option value="off">Yalnız kapalı</option>
            <option value="">Graf ayrımı yok (düz yarış)</option>
          </select>
          <label htmlFor={`${id}-max`}>--max-cost (USD)</label>
          <input id={`${id}-max`} className="text-input g9-num" type="number" min={0} step="any" value={maxCost} onChange={(e) => setMaxCost(e.target.value)} />
        </div>
        <div className="g7-inline">
          <button type="submit" className="btn primary small" disabled={busy || running} data-kor-cmd="bench.estimate">
            <Play size={14} aria-hidden="true" /> {busy ? 'Tahmin ediliyor...' : 'Tahmin et ve çalıştır'}
          </button>
          {running && (
            <button type="button" className="btn small" onClick={() => void cancel()} data-kor-cmd="bench.cancel">
              <Square size={14} aria-hidden="true" /> Durdur
            </button>
          )}
        </div>
        {error && (
          <p className="error-text" role="alert">
            {error}
          </p>
        )}
      </form>

      {running && (
        <div role="status" className="g14-section">
          {progress ? (
            <>
              <div className="bar" role="progressbar" aria-label="Bench ilerlemesi" aria-valuenow={progress.index + (progress.kind === 'done' ? 1 : 0)} aria-valuemin={0} aria-valuemax={progress.total}>
                <span className="bar-fill" style={{ width: `${((progress.index + (progress.kind === 'done' ? 1 : 0)) / Math.max(1, progress.total)) * 100}%` }} />
              </div>
              <span className="g14-sub" translate="no">
                {progress.index + 1}/{progress.total} · {progress.kind === 'start' ? `${progress.model} · ${progress.task}` : `${progress.row.model} · ${progress.row.task}: ${progress.row.passed ? 'geçti' : 'kaldı'}`}
              </span>
            </>
          ) : (
            <span className="g14-sub">Bench çalışıyor...</span>
          )}
        </div>
      )}

      {last?.cancelled && <p className="g7-note">Son çalıştırma durduruldu; sonuçlar eksik.</p>}
      {last?.report ? <Results report={last.report} /> : !running && <p className="muted g14-empty">Henüz bench sonucu yok.</p>}
    </div>
  )
}

const armLabel = (a: BenchSummary['arm']) =>
  a === 'graph-on' ? 'graf açık' : a === 'graph-off' ? 'graf kapalı' : a === 'routed' ? 'yönlendirici' : a === 'baseline' ? 'temel' : ''

function Results({ report }: { report: BenchReport }) {
  return (
    <>
      <section className="g14-section" aria-label="Bench sonuçları">
        <h3 className="g14-h">Sonuçlar</h3>
        <div className="g7-table-wrap">
          <table className="g7-table">
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col" className="num">Geçen</th>
                <th scope="col" className="num">Süre</th>
                <th scope="col" className="num">Token</th>
                <th scope="col" className="num">Maliyet</th>
              </tr>
            </thead>
            <tbody>
              {report.summary.map((s) => (
                <tr key={`${s.model}-${s.arm ?? ''}`}>
                  <td>
                    <span className="g7-mono" translate="no">{s.model}</span> {s.arm && s.arm !== 'race' && <span className="g14-tag">{armLabel(s.arm)}</span>}
                  </td>
                  <td className="num">
                    {s.passed}/{s.tasks} (%{Math.round(s.pass_rate * 100)})
                  </td>
                  <td className="num">{formatMs(s.wall_ms)}</td>
                  <td className="num">{formatTokens(s.input_tokens + s.output_tokens)}</td>
                  <td className="num">{money(s.cost_usd, s.cost_estimated, 4)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <details>
          <summary>Görev satırları ({report.rows.length})</summary>
          <ul className="g14-list">
            {report.rows.map((r, i) => (
              <li key={`${r.task}-${r.model}-${r.arm}-${i}`} translate="no">
                {r.passed ? '✓' : '✗'} {r.task} · {r.model} {r.arm !== 'race' ? `(${r.arm})` : ''} · {formatMs(r.wall_ms)} · {money(r.cost_usd, r.cost_estimated, 4)}
                {r.error && <span className="error-text"> {r.error}</span>}
              </li>
            ))}
          </ul>
        </details>
      </section>

      {report.graph && (
        <section className="g14-section" aria-label="Graf açık/kapalı karşılaştırması">
          <h3 className="g14-h">Graf açık / kapalı</h3>
          {report.graph.meets_acceptance !== null && (
            <span className={`g14-tag ${report.graph.meets_acceptance ? '' : 'warn'}`}>
              {report.graph.meets_acceptance ? 'Hedef tuttu: en az %50 daha az token' : 'Hedef tutmadı (en az %50 daha az token, en fazla 5 puan düşüş)'}
            </span>
          )}
          {graphMetrics(report.graph).map((m) => (
            <div key={m.label}>
              <p className="g14-sub">
                {m.label}
                {m.delta !== null ? ` · ${m.delta}` : ''}
              </p>
              <HBars
                label={`${m.label}: graf kapalı ve açık`}
                items={[
                  { key: 'Graf kapalı', value: m.off ?? 0, text: m.offText },
                  { key: 'Graf açık', value: m.on ?? 0, text: m.onText },
                ]}
              />
            </div>
          ))}
        </section>
      )}

      {report.comparison && (
        <p className="g7-note">
          Yönlendirici açık: maliyet {report.comparison.cost_delta_pct === null ? '?' : `${report.comparison.cost_delta_pct.toFixed(1)}%`}, geçme {report.comparison.pass_delta_pp.toFixed(1)} puan
          {report.comparison.meets_acceptance === true ? ' (hedef tuttu)' : report.comparison.meets_acceptance === false ? ' (hedef tutmadı)' : ''}.
        </p>
      )}

      {report.routing_toml && (
        <section className="g14-section" aria-label="Önerilen yönlendirme">
          <h3 className="g14-h">Önerilen [routing]</h3>
          <pre className="g14-pre" translate="no">{report.routing_toml}</pre>
        </section>
      )}
    </>
  )
}
