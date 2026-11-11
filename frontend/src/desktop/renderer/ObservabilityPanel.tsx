// [g14] "Gözlem" penceresi: genel durum, izler, araçlar, maliyet, olay akışı, süreçler, ham istekler, ayarlar.
import { ThumbsDown, ThumbsUp } from 'lucide-react'
import { useId, useMemo, useState, type ReactNode } from 'react'
import useSWR from 'swr'
import type { ErrorBucket, Overview, TelemetryEvent, TurnRecord } from '../ipc-g14.ts'
import { api, errorMessage } from './api.ts'
import type { G14Tab } from './g14-bus.ts'
import { Columns, HBars, statusName, Timeline } from './g14-charts.tsx'
import { formatClock, formatDayTime, formatMb, formatMs, formatPercent, formatTokens, formatUsd } from './g14-format.ts'
import { G7Modal } from './G7Modal.tsx'
import { Select } from './kor-ui/index.ts'
import { CostTab, ProcessesTab, RawTab, SettingsTab } from './ObservabilityExtra.tsx'

const TABS: ReadonlyArray<{ id: G14Tab; label: string }> = [
  { id: 'overview', label: 'Genel' },
  { id: 'traces', label: 'İzler' },
  { id: 'tools', label: 'Araçlar' },
  { id: 'cost', label: 'Maliyet' },
  { id: 'events', label: 'Olaylar' },
  { id: 'processes', label: 'Süreçler' },
  { id: 'raw', label: 'Ham istekler' },
  { id: 'settings', label: 'Ayarlar' },
]

interface PanelProps {
  open: boolean
  tab: G14Tab
  onTab: (tab: G14Tab) => void
  onClose: () => void
  sessionId: string | null
  onModelChange: (model: string) => void
}

export function ObservabilityPanel({ open, tab, onTab, onClose, sessionId, onModelChange }: PanelProps) {
  const titleId = useId()
  const needsOverview = open && (tab === 'overview' || tab === 'traces' || tab === 'tools' || tab === 'events')
  const overview = useSWR(needsOverview ? 'g14:overview' : null, () => api.g14.overview(), { refreshInterval: 2_000, revalidateOnFocus: false })

  return (
    <G7Modal open={open} onClose={onClose} titleId={titleId} title="Gözlem" className="g7-full g14-modal">
      <div className="g14-tabs" role="tablist" aria-label="Gözlem bölümleri">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`${titleId}-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`${titleId}-panel`}
            className={`g14-tab${tab === t.id ? ' active' : ''}`}
            onClick={() => onTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="g14-body" role="tabpanel" id={`${titleId}-panel`} aria-labelledby={`${titleId}-${tab}`}>
        {needsOverview && overview.error && <p className="error-text" role="alert">{errorMessage(overview.error)}</p>}
        {needsOverview && !overview.data && !overview.error && <p className="muted">Yükleniyor...</p>}
        {overview.data && tab === 'overview' && <OverviewTab data={overview.data} onTab={onTab} />}
        {overview.data && tab === 'traces' && <TracesTab data={overview.data} sessionId={sessionId} onRated={() => void overview.mutate()} />}
        {overview.data && tab === 'tools' && <ToolsTab data={overview.data} />}
        {overview.data && tab === 'events' && <EventsTab events={overview.data.events} sessionId={sessionId} />}
        {open && tab === 'cost' && <CostTab sessionId={sessionId} onModelChange={onModelChange} />}
        {open && tab === 'processes' && <ProcessesTab />}
        {open && tab === 'raw' && <RawTab />}
        {open && tab === 'settings' && <SettingsTab />}
      </div>
    </G7Modal>
  )
}

// ---------- Genel ----------

function errorPoints(errors: readonly ErrorBucket[], source: 'tool' | 'model') {
  return errors.map((b) => {
    const calls = source === 'tool' ? b.toolCalls : b.modelCalls
    const failed = source === 'tool' ? b.toolErrors : b.modelErrors
    const label = formatClock(b.start).slice(0, 5)
    return {
      label,
      value: calls === 0 ? null : failed / calls,
      title: calls === 0 ? `${label}: çağrı yok` : `${label}: ${failed}/${calls} hata (${formatPercent(failed / calls)})`,
    }
  })
}

function OverviewTab({ data, onTab }: { data: Overview; onTab: (tab: G14Tab) => void }) {
  const [probing, setProbing] = useState(false)
  const [probeError, setProbeError] = useState<string | null>(null)
  const { data: probed, mutate } = useSWR('g14:probe', () => api.g14.probe(), { revalidateOnFocus: false, refreshInterval: 60_000 })
  const health = probed && probed.probe && (!data.health.probe || probed.probe.at >= data.health.probe.at) ? { ...data.health, probe: probed.probe } : data.health
  const p = health.provider
  const memRatio = health.memoryMb / Math.max(1, health.memoryBudgetMb)

  const runProbe = async () => {
    setProbing(true)
    setProbeError(null)
    try {
      await mutate(api.g14.probe(), { revalidate: false })
    } catch (err) {
      setProbeError(errorMessage(err))
    } finally {
      setProbing(false)
    }
  }

  return (
    <>
      <section className="g14-section" aria-label="Canlı sağlık">
        <h3 className="g14-h">Canlı sağlık</h3>
        <div className="g14-cards">
          <Badge title="Ağ" state={health.online ? 'ok' : 'down'} value={health.online ? 'Çevrimiçi' : 'Çevrimdışı'} detail="İşletim sistemi bildirimi" />
          <Badge
            title={`Sağlayıcı (${p.host})`}
            state={p.state}
            value={{ ok: 'Sağlıklı', degraded: 'Yavaşlama/hata', down: 'Hata veriyor', unknown: 'Veri yok' }[p.state]}
            detail={p.recentCalls > 0 ? `Son 10 dk: ${p.recentCalls} çağrı, hata ${formatPercent(p.errorRate)}, p95 ${formatMs(p.p95Ms ?? 0)}` : 'Son 10 dakikada model çağrısı yok'}
          />
          <Badge
            title="Erişilebilirlik denetimi"
            state={health.probe === null ? 'unknown' : health.probe.ok ? 'ok' : 'down'}
            value={health.probe === null ? 'Denetlenmedi' : health.probe.ok ? `${formatMs(health.probe.latencyMs)}` : 'Ulaşılamadı'}
            detail={health.probe ? `${formatClock(health.probe.at)}${health.probe.status !== null ? `, HTTP ${health.probe.status}` : ''}` : 'Kimlik bilgisi gönderilmez'}
            action={
              <button type="button" className="btn small" onClick={() => void runProbe()} disabled={probing}>
                {probing ? 'Denetleniyor...' : 'Şimdi denetle'}
              </button>
            }
          />
          <Badge
            title="Bellek (ana süreç)"
            state={memRatio > 1 ? 'down' : memRatio > 0.8 ? 'degraded' : 'ok'}
            value={formatMb(health.memoryMb)}
            detail={`Bütçe ${formatMb(health.memoryBudgetMb)}`}
          />
        </div>
        {probeError && <p className="error-text" role="alert">{probeError}</p>}
      </section>

      <section className="g14-section" aria-label="Performans bütçesi">
        <h3 className="g14-h">Süren turlar</h3>
        {data.live.length === 0 ? (
          <p className="muted ks-empty">Şu an süren tur yok.</p>
        ) : (
          <ul className="g14-list">
            {data.live.map((l) => {
              const over = !l.subagent && l.elapsedMs > data.settings.turnBudgetMs
              return (
                <li key={`${l.traceId}-${l.start}`} className={over ? 'g14-over' : undefined}>
                  <span>{l.subagent ? `Alt ajan: ${l.subagent}` : 'Tur'}</span>
                  <span className="g14-num">{formatMs(l.elapsedMs)}</span>
                  {over && <span className="g14-tag warn">Bütçe aşıldı ({formatMs(data.settings.turnBudgetMs)})</span>}
                </li>
              )
            })}
          </ul>
        )}
      </section>

      <section className="g14-section" aria-label="Hata oranı">
        <h3 className="g14-h">Hata oranı (son 60 dakika, 5 dakikalık aralıklar)</h3>
        <div className="g14-grid2">
          <div>
            <p className="g14-sub">Araçlar</p>
            <Columns points={errorPoints(data.errors, 'tool')} max={1} label="Araç hata oranı" format={formatPercent} />
          </div>
          <div>
            <p className="g14-sub">Sağlayıcı (model çağrıları)</p>
            <Columns points={errorPoints(data.errors, 'model')} max={1} label="Sağlayıcı hata oranı" format={formatPercent} />
          </div>
        </div>
      </section>

      <section className="g14-section" aria-label="Yavaş araçlar">
        <h3 className="g14-h">Yavaş araç çağrıları (eşik {formatMs(data.settings.slowToolMs)})</h3>
        {data.slow.length === 0 ? (
          <p className="muted ks-empty">Eşiği aşan araç çağrısı yok.</p>
        ) : (
          <ul className="g14-slow">
            {data.slow.map((s) => (
              <li key={s.id}>
                <div className="g14-slow-head">
                  <strong>{s.name}</strong>
                  <span className="g14-num">{formatMs(s.end - s.start)}</span>
                  <span className="muted">{formatDayTime(s.start)}</span>
                  {s.status !== 'ok' && <span className="g14-tag err">{statusName(s.status)}</span>}
                </div>
                {s.summary && <code className="g14-code truncate">{s.summary}</code>}
                <p className="g14-hint">{s.hint}</p>
              </li>
            ))}
          </ul>
        )}
        <button type="button" className="btn small ghost" onClick={() => onTab('tools')}>
          Tüm araç gecikmeleri
        </button>
      </section>
    </>
  )
}

function Badge({ title, state, value, detail, action }: { title: string; state: 'ok' | 'degraded' | 'down' | 'unknown'; value: string; detail: string; action?: ReactNode }) {
  const word = { ok: 'iyi', degraded: 'uyarı', down: 'sorun', unknown: 'bilinmiyor' }[state]
  return (
    <div className="g14-card">
      <span className="g14-card-title">{title}</span>
      <span className="g14-card-value">
        <span className={`g14-dot ${state}`} aria-hidden="true" />
        {value}
        <span className="sr-only"> ({word})</span>
      </span>
      <span className="g14-card-detail">{detail}</span>
      {action}
    </div>
  )
}

// ---------- İzler ----------

function TracesTab({ data, sessionId, onRated }: { data: Overview; sessionId: string | null; onRated: () => void }) {
  const [onlySession, setOnlySession] = useState(sessionId !== null)
  const [selected, setSelected] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const turns = data.turns.filter((t) => !onlySession || t.sessionId === sessionId)
  const turn = turns.find((t) => `${t.traceId}:${t.spanId}` === selected) ?? turns[0]
  const children = turn ? data.turns.filter((t) => t.traceId === turn.traceId && t.spanId !== turn.spanId) : []

  const rate = async (t: TurnRecord, rating: 1 | -1) => {
    setError(null)
    try {
      await api.g14.rate(t.traceId, data.ratings[t.traceId] === rating ? 0 : rating)
      onRated()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  return (
    <div className="g14-split">
      <section className="g14-section" aria-label="Turlar">
        <h3 className="g14-h">Son turlar</h3>
        <label className="g14-check">
          <input type="checkbox" checked={onlySession} disabled={sessionId === null} onChange={(e) => setOnlySession(e.target.checked)} />
          Yalnızca bu sohbet
        </label>
        {turns.length === 0 ? (
          <p className="muted ks-empty">Kayıtlı tur yok. İzler uygulama açıkken bellekte tutulur.</p>
        ) : (
          <ul className="g14-turns" aria-label="Son turlar">
            {turns.map((t) => {
              const key = `${t.traceId}:${t.spanId}`
              return (
                <li key={key}>
                  <button type="button" className={`g14-turn${turn && key === `${turn.traceId}:${turn.spanId}` ? ' active' : ''}`} onClick={() => setSelected(key)} aria-current={turn === t}>
                    <span className="g14-turn-top">
                      <span>{formatClock(t.start)}</span>
                      <span className="truncate">{t.subagent ? `Alt ajan: ${t.subagent}` : t.project}</span>
                      {t.status !== 'ok' && <span className={`g14-tag ${t.status === 'error' ? 'err' : 'warn'}`}>{statusName(t.status)}</span>}
                    </span>
                    <span className="g14-turn-meta">
                      {formatMs(t.end - t.start)} · {formatTokens(t.tokens.system + t.tokens.toolDefs + t.tokens.toolResults + t.tokens.history + t.tokens.completion)} token · {formatUsd(t.costUsd)}
                    </span>
                  </button>
                </li>
              )
            })}
          </ul>
        )}
      </section>

      {turn && (
        <section className="g14-section g14-detail" aria-label="Tur ayrıntısı">
          <div className="g14-detail-head">
            <div>
              <h3 className="g14-h">{turn.subagent ? `Alt ajan: ${turn.subagent}` : 'Tur'} · {turn.model}</h3>
              <p className="g14-sub">
                {formatDayTime(turn.start)} · {formatMs(turn.end - turn.start)} · {turn.modelCalls} model, {turn.toolCalls} araç çağrısı · iz <code>{turn.traceId.slice(0, 8)}</code>
                {turn.message ? ` · ${turn.message}` : ''}
              </p>
            </div>
            {!turn.subagent && (
              <div className="g14-rate-group" role="group" aria-label="Yanıt kalitesi">
                <button type="button" className={`action-btn g14-rate${data.ratings[turn.traceId] === 1 ? ' active' : ''}`} aria-pressed={data.ratings[turn.traceId] === 1} aria-label="Bu turu beğen" onClick={() => void rate(turn, 1)}>
                  <ThumbsUp size={16} aria-hidden="true" />
                </button>
                <button type="button" className={`action-btn g14-rate${data.ratings[turn.traceId] === -1 ? ' active' : ''}`} aria-pressed={data.ratings[turn.traceId] === -1} aria-label="Bu turu beğenme" onClick={() => void rate(turn, -1)}>
                  <ThumbsDown size={16} aria-hidden="true" />
                </button>
              </div>
            )}
          </div>
          {error && <p className="error-text" role="alert">{error}</p>}

          <h4 className="g14-h4">Adım zaman çizelgesi</h4>
          <Timeline steps={turn.steps} start={turn.start} end={turn.end} />

          <h4 className="g14-h4">Token dağılımı ({formatTokens(turn.tokens.cached)} önbellekten)</h4>
          <HBars
            label="Tur token dağılımı"
            items={[
              { key: 'Sistem istemi', value: turn.tokens.system },
              { key: 'Araç tanımları', value: turn.tokens.toolDefs },
              { key: 'Araç sonuçları', value: turn.tokens.toolResults },
              { key: 'Geçmiş (konuşma)', value: turn.tokens.history },
              { key: 'Yanıt (çıktı)', value: turn.tokens.completion },
            ].map((i) => ({ ...i, text: formatTokens(i.value) }))}
          />
          <p className="g14-note">İstem kırılımı karakter tahminidir; toplamı sağlayıcının bildirdiği prompt_tokens değerine ölçeklenir.</p>

          {children.length > 0 && (
            <>
              <h4 className="g14-h4">Bu izdeki alt ajanlar</h4>
              <ul className="g14-list">
                {children.map((c) => (
                  <li key={c.spanId}>
                    <button type="button" className="btn small ghost" onClick={() => setSelected(`${c.traceId}:${c.spanId}`)}>
                      {c.subagent ?? 'alt ajan'} · {c.model}
                    </button>
                    <span className="g14-num">{formatMs(c.end - c.start)}</span>
                    <span className="g14-num">{formatUsd(c.costUsd)}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>
      )}
    </div>
  )
}

// ---------- Araçlar ----------

function ToolsTab({ data }: { data: Overview }) {
  return (
    <>
      <section className="g14-section" aria-label="Araç kullanımı ve gecikme">
        <h3 className="g14-h">Araç kullanımı ve gecikme</h3>
        {data.tools.length === 0 ? (
          <p className="muted ks-empty">Henüz araç çağrısı yok.</p>
        ) : (
          <div className="g7-table-wrap">
            <table className="g7-table">
              <thead>
                <tr>
                  <th scope="col">Araç</th>
                  <th scope="col" className="num">Çağrı</th>
                  <th scope="col" className="num">Başarı</th>
                  <th scope="col" className="num">Hata</th>
                  <th scope="col" className="num">Reddedilen</th>
                  <th scope="col" className="num">p50</th>
                  <th scope="col" className="num">p95</th>
                  <th scope="col" className="num">En uzun</th>
                  <th scope="col" className="num">Yavaş</th>
                </tr>
              </thead>
              <tbody>
                {data.tools.map((t) => (
                  <tr key={t.name}>
                    <th scope="row">{t.name}</th>
                    <td className="num">{t.count}</td>
                    <td className="num">{formatPercent(t.successRate)}</td>
                    <td className="num">{t.errors}</td>
                    <td className="num">{t.denied}</td>
                    <td className="num">{formatMs(t.p50)}</td>
                    <td className="num">{formatMs(t.p95)}</td>
                    <td className="num">{formatMs(t.max)}</td>
                    <td className="num">{t.slow}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <HBars label="Araç çağrı sayıları" items={data.tools.map((t) => ({ key: t.name, value: t.count, text: `${t.count} çağrı` }))} limit={12} />
      </section>

      <section className="g14-section" aria-label="Model çağrıları">
        <h3 className="g14-h">Model çağrıları</h3>
        {data.models.length === 0 ? (
          <p className="muted ks-empty">Henüz model çağrısı yok.</p>
        ) : (
          <div className="g7-table-wrap">
            <table className="g7-table">
              <thead>
                <tr>
                  <th scope="col">Model</th>
                  <th scope="col" className="num">Çağrı</th>
                  <th scope="col" className="num">Hata</th>
                  <th scope="col" className="num">İlk token (p50)</th>
                  <th scope="col" className="num">p50</th>
                  <th scope="col" className="num">p95</th>
                  <th scope="col" className="num">Girdi</th>
                  <th scope="col" className="num">Önbellek</th>
                  <th scope="col" className="num">Çıktı</th>
                </tr>
              </thead>
              <tbody>
                {data.models.map((m) => (
                  <tr key={m.model}>
                    <th scope="row">{m.model}</th>
                    <td className="num">{m.count}</td>
                    <td className="num">{m.errors}</td>
                    <td className="num">{m.ttftP50 === null ? '-' : formatMs(m.ttftP50)}</td>
                    <td className="num">{formatMs(m.p50)}</td>
                    <td className="num">{formatMs(m.p95)}</td>
                    <td className="num">{formatTokens(m.promptTokens)}</td>
                    <td className="num">{formatTokens(m.cachedTokens)}</td>
                    <td className="num">{formatTokens(m.completionTokens)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </>
  )
}

// ---------- Olay akışı ----------

const KIND_LABEL: Record<TelemetryEvent['kind'], string> = { turn: 'Tur', model: 'Model', tool: 'Araç', subagent: 'Alt ajan', alert: 'Uyarı', compact: 'Sıkıştırma' }

function EventsTab({ events, sessionId }: { events: readonly TelemetryEvent[]; sessionId: string | null }) {
  const [kind, setKind] = useState<'all' | TelemetryEvent['kind']>('all')
  const [problems, setProblems] = useState(false)
  const [onlySession, setOnlySession] = useState(false)
  const [query, setQuery] = useState('')
  const shown = useMemo(() => {
    const q = query.trim().toLocaleLowerCase('tr-TR')
    return events.filter(
      (e) =>
        (kind === 'all' || e.kind === kind) &&
        (!problems || (e.status !== undefined && e.status !== 'ok') || e.kind === 'alert') &&
        (!onlySession || e.sessionId === sessionId) &&
        (!q || e.label.toLocaleLowerCase('tr-TR').includes(q)),
    )
  }, [events, kind, problems, onlySession, sessionId, query])

  return (
    <section className="g14-section" aria-label="Olay akışı">
      <h3 className="g14-h">Olay akışı</h3>
      <div className="g14-filters">
        <Select
          label="Tür"
          hideLabel
          value={kind}
          options={[{ value: 'all' as const, label: 'Tümü' }, ...(Object.entries(KIND_LABEL) as Array<[TelemetryEvent['kind'], string]>).map(([value, label]) => ({ value, label }))]}
          onChange={setKind}
        />
        <label className="g14-check">
          <input type="checkbox" checked={problems} onChange={(e) => setProblems(e.target.checked)} />
          Yalnızca sorunlar
        </label>
        <label className="g14-check">
          <input type="checkbox" checked={onlySession} disabled={sessionId === null} onChange={(e) => setOnlySession(e.target.checked)} />
          Yalnızca bu sohbet
        </label>
        <input type="search" className="g14-input" placeholder="Olaylarda ara" aria-label="Olaylarda ara" value={query} maxLength={200} onChange={(e) => setQuery(e.target.value)} />
        <span className="muted g14-num">{shown.length} olay</span>
      </div>
      {shown.length === 0 ? (
        <p className="muted ks-empty">Filtreye uyan olay yok.</p>
      ) : (
        <div className="g7-table-wrap">
          <table className="g7-table g14-events" aria-label="Olaylar (en yeni üstte)">
            <thead>
              <tr>
                <th scope="col">Saat</th>
                <th scope="col">Tür</th>
                <th scope="col">Olay</th>
                <th scope="col" className="num">Süre</th>
                <th scope="col">Durum</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((e) => (
                <tr key={e.id} className={e.status && e.status !== 'ok' ? 'problem' : e.kind === 'alert' ? 'alert' : undefined}>
                  <td className="g14-num">
                    <time dateTime={new Date(e.at).toISOString()}>{formatClock(e.at)}</time>
                  </td>
                  <td className="g14-ev-kind">{KIND_LABEL[e.kind]}</td>
                  <td className="g14-ev-label">
                    <code className="truncate" title={e.label}>
                      {e.phase === 'start' ? 'Başladı: ' : ''}
                      {e.label}
                    </code>
                  </td>
                  <td className="num">{e.durationMs !== undefined ? formatMs(e.durationMs) : ''}</td>
                  <td>{e.status && e.status !== 'ok' ? <span className="g14-tag err">{statusName(e.status)}</span> : e.kind === 'alert' ? <span className="g14-tag warn">{KIND_LABEL.alert}</span> : ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  )
}
