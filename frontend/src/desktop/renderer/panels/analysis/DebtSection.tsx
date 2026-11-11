// Teknik borç (§9 `kor debt`, ADR 0047): dosya başına tahmini saat (≈), formül, aylık artış ve eşik projeksiyonu.
import { Hourglass } from 'lucide-react'
import { useId, useState } from 'react'
import type { Debt } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { metricsApi } from './metrics-bridge.ts'
import { DEBT_FORMULA, growthBasis, num, testBasis } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

const SHOWN = 30

function DebtSection() {
  const id = useId()
  const [threshold, setThreshold] = useState('')
  const [debt, setDebt] = useState<Debt | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      const t = Number.parseInt(threshold, 10)
      setDebt(await metricsApi.debt(Number.isFinite(t) && t > 0 ? t : null))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="km">
      <p className="km-formula">{`Formül: ${DEBT_FORMULA}`}</p>
      <div className="km-row">
        <label className="km-field" htmlFor={`${id}-th`}>
          <span>Eşik (saat; boş: yapılandırma ya da 400)</span>
          <input id={`${id}-th`} inputMode="numeric" value={threshold} onChange={(e) => setThreshold(e.target.value.replace(/\D/g, '').slice(0, 7))} />
        </label>
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void run()} data-kor-cmd="metrics.debt">
          <Hourglass size={14} aria-hidden="true" /> {busy ? 'Hesaplanıyor…' : 'Borcu hesapla'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {debt && (
        <>
          <p role="status">{`Toplam: ≈ ${num(debt.total_hours)} sa, ${debt.items.length} dosyada (${debt.files_scanned} dosya tarandı)`}</p>
          <p>{debt.growth ? `Aylık artış: ≈ ${num(debt.growth.per_month)} sa (${growthBasis(debt.growth.basis)})` : 'Aylık artış: veri yok (30 gün öncesine geçmiş ve 14 gün arayla iki anlık görüntü yok).'}</p>
          <p>
            {debt.months_to_threshold === null
              ? `${num(debt.threshold_hours)} sa eşiği bu hızla aşılmıyor.`
              : debt.months_to_threshold <= 0
                ? `${num(debt.threshold_hours)} sa eşiği zaten aşıldı.`
                : `${num(debt.threshold_hours)} sa eşiği bu hızla ≈ ${num(debt.months_to_threshold)} ayda aşılır.`}
          </p>
          {debt.items.length === 0 ? (
            <p className="muted">Bu formüle göre borç yok.</p>
          ) : (
            <table className="km-table">
              <thead>
                <tr><th scope="col">Dosya</th><th scope="col">Saat</th><th scope="col">Ayrıntı</th></tr>
              </thead>
              <tbody>
                {debt.items.slice(0, SHOWN).map((i) => (
                  <tr key={i.file}>
                    <td>
                      <button type="button" className="km-loc" onClick={() => g3Actions.current?.openFile(i.file, i.worst?.[1])} translate="no">{i.file}</button>
                    </td>
                    <td>{`≈ ${num(i.hours)}`}</td>
                    <td className="km-src">
                      {`10’u aşan karmaşıklık ${i.complexity_excess}, ${i.code_lines} kod satırı, ${i.duplicated_lines} tekrarlanan, 180 günde ${i.churn} commit, ${testBasis(i.test)}`}
                      {i.worst && ` · en kötüsü ${i.worst[0]} (satır ${i.worst[1]}, ${i.worst[2]})`}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </div>
  )
}

registerSection({ id: 'debt', label: 'Teknik borç', component: DebtSection })
