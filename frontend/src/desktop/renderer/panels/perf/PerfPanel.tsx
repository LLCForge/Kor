// Performans profili (§15.8, ADR 0049): bir komutu, testleri ya da bir HTTP ucunu dile uygun profiler ile
// çalıştırır; flamegraph (betiksiz SVG, <img> olarak), proje grafına eşlenen en sıcak fonksiyonlar, bellek
// profili (varsa), ajanla iyileştirme ve aynı ölçümle öncesi/sonrası tablosu. Sayılar yalnızca ölçümden gelir.
// CLI: kor perf <hedef> | kor perf report <dosya> | kor perf list.
import { FileSearch, Gauge, RefreshCw, RotateCcw, Sparkles } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { PerfReport, PerfRow } from '../../../ipc-perf.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { insertIntoDraft } from '../../g21/quote.ts'
import type { KorPanelProps } from '../registry.ts'
import { perfApi } from './api.ts'
import { EMPTY_FORM, PROFILERS, changeText, changeTone, isEndpoint, location, metricLabel, num, openable, pct, svgDataUrl, toOptions, weight, type PerfForm } from './format.ts'

interface Loaded {
  report: PerfReport
  flamegraph: string | null
  memoryFlamegraph: string | null
}

function FunctionTable({ rows, caption, limit }: { rows: PerfRow[]; caption: string; limit: number }) {
  return (
    <div className="kp-table-wrap">
      <table className="kp-table">
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr>
            <th scope="col">Kendi</th>
            <th scope="col">Toplam</th>
            <th scope="col">Fonksiyon</th>
            <th scope="col">Konum</th>
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, limit).map((r, i) => {
            const file = r.node?.file ?? r.file
            const line = r.node?.line ?? r.line
            return (
              <tr key={`${r.name}-${i}`}>
                <td className="kp-num">{pct(r.selfPct)}</td>
                <td className="kp-num">{pct(r.totalPct)}</td>
                <td className="kp-fn" translate="no" title={r.node ? `Graf düğümü: ${r.node.id}` : undefined}>
                  {r.name}
                  {r.node && <span className="kp-node">{r.node.by === 'line' ? 'graf · satır' : 'graf · ad'}</span>}
                </td>
                <td translate="no">
                  {openable(file) ? (
                    <button type="button" className="kg-link kp-loc" onClick={() => g3Actions.current?.openFile(file, line ?? undefined)} title="Dosyayı önizlemede aç">
                      {location({ file, line })}
                    </button>
                  ) : (
                    <span className="muted kp-loc">{location({ file, line })}</span>
                  )}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function ReportView({ data, onRemeasure, busy }: { data: Loaded; onRemeasure: (r: PerfReport) => void; busy: boolean }) {
  const uid = useId()
  const r = data.report
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const optimize = () => {
    setNote(null)
    perfApi
      .optimizePrompt(r.id)
      .then((p) => {
        if (p) {
          insertIntoDraft(p)
          setNote({ text: 'Görev mesaj kutusuna yazıldı. Gönderdiğinizde ajan değişikliği önerir; her dosya yazımı ayrıca onay ister. Sonra "Yeniden ölç" ile aynı ölçümü tekrarlayın.', ok: true })
        } else {
          setNote({ text: 'Hiçbir sıcak fonksiyon bu projenin koduna eşlenmedi; ajanın iyileştireceği bir şey yok.', ok: false })
        }
      })
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
  }
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <div className="kg-card-head">
        <h3 id={`${uid}-h`} className="kg-card-title ks-title">
          <span className="kg-card-name" translate="no">{r.target}</span>
        </h3>
        {r.kind !== 'file' && (
          <button type="button" className="action-btn" disabled={busy} onClick={() => onRemeasure(r)} aria-label="Aynı ölçümü tekrarla ve karşılaştır" title="Yeniden ölç" data-kor-cmd="perf.run">
            <RotateCcw size={16} aria-hidden="true" />
          </button>
        )}
      </div>
      <p className="muted kp-small" translate="no">
        {[
          r.profiler,
          r.language,
          r.wallMs !== null ? `${r.wallMs.toFixed(0)} ms` : null,
          weight(r.total, r.unit),
          r.exitCode !== null ? `çıkış ${r.exitCode}` : null,
          `korumalı alan: ${r.sandbox}`,
        ]
          .filter(Boolean)
          .join(' · ')}
      </p>
      {r.load && (
        <p className="kp-small">
          {`Yük: ${r.load.requests} istek (${r.load.ok} başarılı, ${r.load.errors} hata) · ${r.load.rps.toFixed(1)} istek/sn · p50 ${r.load.p50Ms.toFixed(1)} ms · p95 ${r.load.p95Ms.toFixed(1)} ms`}
          {r.load.firstError && <span className="error-text" translate="no">{` · ${r.load.firstError}`}</span>}
        </p>
      )}
      {r.hottest?.node && (
        <p className="kp-hot">{`En sıcak proje fonksiyonu ${r.hottest.node.name}: kendi başına ${pct(r.hottest.selfPct)}, çağırdıklarıyla ${pct(r.hottest.totalPct)}.`}</p>
      )}
      {r.compare.length > 0 && (
        <div className="kp-table-wrap">
          <table className="kp-table">
            <caption className="kp-caption">Öncesi / sonrası (ölçülen)</caption>
            <thead>
              <tr>
                <th scope="col">Ölçüt</th>
                <th scope="col">Önce</th>
                <th scope="col">Sonra</th>
                <th scope="col">Değişim</th>
              </tr>
            </thead>
            <tbody>
              {r.compare.map((c) => (
                <tr key={`${c.metric}-${c.subject ?? ''}`}>
                  <td translate="no">{metricLabel(c)}</td>
                  <td className="kp-num">{num(c.before)}</td>
                  <td className="kp-num">{num(c.after)}</td>
                  <td className={`kp-num kp-${changeTone(c)}`}>{changeText(c)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {data.flamegraph ? (
        <figure className="kp-flame">
          <img src={svgDataUrl(data.flamegraph)} alt={`${r.target} flamegraph: en geniş kutular en çok zaman alan çağrı yolları; ayrıntılar aşağıdaki tabloda`} />
        </figure>
      ) : (
        <p className="muted">Flamegraph dosyası okunamadı.</p>
      )}
      <h4 className="kp-h">En çok zaman alan fonksiyonlar</h4>
      <FunctionTable rows={r.top} caption="En çok zaman alan fonksiyonlar" limit={15} />
      {r.memory ? (
        <>
          <h4 className="kp-h">{`Bellek (ayrılan): ${weight(r.memory.total, r.memory.unit)}${r.memory.count !== null ? ` · ${r.memory.count} ayırma` : ''}`}</h4>
          {data.memoryFlamegraph && (
            <figure className="kp-flame">
              <img src={svgDataUrl(data.memoryFlamegraph)} alt="Bellek ayırma flamegraph'ı" />
            </figure>
          )}
          <FunctionTable rows={r.memory.top} caption="En çok bellek ayıran fonksiyonlar" limit={8} />
        </>
      ) : (
        r.memoryNote && <p className="ks-note" translate="no">{r.memoryNote}</p>
      )}
      {r.notes.length > 0 && (
        <ul className="kp-notes" translate="no">
          {r.notes.map((n, i) => <li key={i}>{n}</li>)}
        </ul>
      )}
      <div className="kg-row">
        <button type="button" className="btn small" onClick={optimize} disabled={!r.hottest?.node} data-kor-cmd="perf.optimizePrompt">
          <Sparkles size={14} aria-hidden="true" />
          Ajanla iyileştir
        </button>
        <span className="muted kp-small" translate="no">{r.flamegraph}</span>
      </div>
      {note && <p className={note.ok ? 'ks-ok' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
    </section>
  )
}

export function PerfPanel(_props: KorPanelProps) {
  const uid = useId()
  const runs = useSWR('perf:list', () => perfApi.list(), { revalidateOnFocus: false })
  const [form, setForm] = useState<PerfForm>(EMPTY_FORM)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [current, setCurrent] = useState<Loaded | null>(null)
  const set = (k: keyof PerfForm) => (e: { target: { value: string } }) => setForm((f) => ({ ...f, [k]: e.target.value }))
  const endpoint = isEndpoint(form.target)

  const act = async (f: () => Promise<PerfReport | null>) => {
    setBusy(true)
    setError(null)
    try {
      const r = await f()
      if (r) setCurrent(await perfApi.load(r.id))
      void runs.mutate()
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }
  const submit = (e: FormEvent) => {
    e.preventDefault()
    void act(() => perfApi.run(toOptions(form)))
  }
  const remeasure = (r: PerfReport) => {
    const f = { ...form, target: r.target, profiler: r.profiler }
    setForm(f)
    void act(() => perfApi.run(toOptions(f, r.id)))
  }
  const open = (id: string) => {
    setError(null)
    perfApi.load(id).then(setCurrent).catch((e: unknown) => setError(errorMessage(e)))
  }

  return (
    <div className="kp">
      <section className="kg-card" aria-labelledby={`${uid}-run`}>
        <h2 id={`${uid}-run`} className="kg-card-title ks-title">
          <Gauge size={16} aria-hidden="true" />
          Performans profili
        </h2>
        <form className="kp-form" onSubmit={submit}>
          <label className="kg-field kp-grow">
            <span>Hedef</span>
            <input value={form.target} onChange={set('target')} placeholder="cargo test, test, npm run bench ya da http://127.0.0.1:3000/api" spellCheck={false} maxLength={1000} translate="no" required />
          </label>
          <label className="kg-field">
            <span>Profiler</span>
            <select value={form.profiler} onChange={set('profiler')}>
              <option value="">Dile göre</option>
              {PROFILERS.map((p) => <option key={p} value={p} translate="no">{p}</option>)}
            </select>
          </label>
          <label className="kg-field kp-narrow">
            <span>{endpoint ? 'Örnekleme (sn)' : 'Süre sınırı (sn)'}</span>
            <input value={form.duration} onChange={set('duration')} inputMode="numeric" placeholder={endpoint ? '10' : '600'} />
          </label>
          {endpoint && (
            <>
              <label className="kg-field kp-narrow">
                <span>İstek</span>
                <input value={form.requests} onChange={set('requests')} inputMode="numeric" />
              </label>
              <label className="kg-field kp-narrow">
                <span>Eşzamanlı</span>
                <input value={form.concurrency} onChange={set('concurrency')} inputMode="numeric" />
              </label>
              <label className="kg-field kp-narrow">
                <span>Sunucu PID</span>
                <input value={form.pid} onChange={set('pid')} inputMode="numeric" />
              </label>
              <label className="kg-field kp-grow">
                <span>ya da başlatma komutu</span>
                <input value={form.cmd} onChange={set('cmd')} placeholder="node server.js" spellCheck={false} maxLength={1000} translate="no" />
              </label>
            </>
          )}
          <button type="submit" className="btn small" disabled={busy || !form.target.trim()} data-kor-cmd="perf.run">
            <Gauge size={14} aria-hidden="true" />
            {busy ? 'Profilleniyor…' : 'Profille'}
          </button>
          <button type="button" className="btn small ghost" disabled={busy} onClick={() => void act(() => perfApi.report())} data-kor-cmd="perf.report">
            <FileSearch size={14} aria-hidden="true" />
            Profil dosyası aç
          </button>
        </form>
        <p className="ks-note">
          Profiler kurulu değilse Kor kurmaz, nasıl kurulacağını söyler. Başka bir süreci dışarıdan örnekleyen profiler'lar (samply, perf, py-spy, dotnet-trace) ve uç sunucuları korumalı alan dışında çalışır; önce onay istenir. Yük yalnızca bu bilgisayardaki adreslere gönderilir.
        </p>
        {busy && <p className="muted" role="status">Profilleniyor; komut bitene ya da örnekleme süresi dolana kadar sürer.</p>}
        {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
      </section>

      {current && <ReportView data={current} onRemeasure={remeasure} busy={busy} />}

      <section className="kg-card" aria-labelledby={`${uid}-list`}>
        <div className="kg-card-head">
          <h3 id={`${uid}-list`} className="kg-card-title ks-title">Önceki çalıştırmalar</h3>
          <button type="button" className="action-btn" onClick={() => void runs.mutate()} aria-label="Çalıştırma listesini yenile" title="Yenile" data-kor-cmd="perf.list">
            <RefreshCw size={16} aria-hidden="true" />
          </button>
        </div>
        {runs.error ? (
          <p className="error-text" role="alert">{errorMessage(runs.error)}</p>
        ) : !runs.data ? (
          <p className="muted" role="status" data-kor-cmd="perf.list">Çalıştırmalar okunuyor…</p>
        ) : runs.data.length === 0 ? (
          <p className="ks-empty" data-kor-cmd="perf.list">Bu projede henüz profil yok. Sonuçlar <code>.kor/perf/</code> altına yazılır (git dışı).</p>
        ) : (
          <ul className="ks-list" data-kor-cmd="perf.list">
            {runs.data.map((s) => (
              <li key={s.id}>
                <button type="button" className="kg-link" aria-current={current?.report.id === s.id} onClick={() => open(s.id)} data-kor-cmd="perf.load">
                  <span translate="no">{s.target}</span>
                  <span className="kg-link-meta" translate="no">
                    {[s.profiler, s.wallMs !== null ? `${s.wallMs.toFixed(0)} ms` : null, s.hottest ? `${s.hottest} ${pct(s.hottestPct ?? 0)}` : null].filter(Boolean).join(' · ')}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  )
}
