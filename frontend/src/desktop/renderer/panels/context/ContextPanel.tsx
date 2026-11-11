// Bağlam paneli (§5.3): son isteğin kategori dökümü, eşikler ve zaman içindeki doluluk. Kaynak çekirdeğin yerel
// günlüğü (`context.*`); rakamların hepsi tahmindir (≈). Tur bitince yenilenir.
import { useCallback, useId, useState, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { ContextThresholds } from '../../../ipc-cost.ts'
import { api, errorMessage } from '../../api.ts'
import { Columns, HBars } from '../../g14-charts.tsx'
import { formatTokens } from '../../g7-format.ts'
import type { KorPanelProps } from '../registry.ts'
import { ContextItems } from '../../prefs/ContextItems.tsx' // [ADR 0050] §28.1 satır eylemleri
import { contextApi, keyPrefix, useRefreshOnTurnDone } from '../cost/api.ts'
import { contextParts, contextSeries, contextTotal, percentSetting } from '../cost/shape.ts'
import { percentText } from '../../ui-locale.ts'

const isContextKey = keyPrefix('context:')

export function ContextPanel(props: KorPanelProps) {
  const { mutate } = useSWRConfig()
  const refresh = useCallback(() => void mutate(isContextKey), [mutate])
  useRefreshOnTurnDone(refresh)
  const last = useSWR(['context:last'], () => contextApi.last(), { revalidateOnFocus: false })
  const history = useSWR(['context:history'], () => contextApi.history(200), { revalidateOnFocus: false })

  if (!last.data) return last.error ? <p className="error-text" role="alert">{errorMessage(last.error)}</p> : <p className="muted">Yükleniyor...</p>
  const { breakdown: b, thresholds, warning, text } = last.data
  const warn = thresholds.warn_percent

  return (
    <div className="g2c-panel g2c-body g2c-stack" data-kor-event="kor:event">
      <section className="g14-section" aria-label="Son istek" data-kor-cmd="context.last">
        <h3 className="g14-h">Son istek</h3>
        {!b ? (
          <p className="muted g14-empty">{thresholds.log ? 'Henüz kayıtlı istek yok. Bir sohbet turundan sonra döküm burada görünür.' : 'Bağlam günlüğü kapalı ([context] log = false).'}</p>
        ) : (
          <>
            <p className="g14-sub">
              <span className="g7-mono" translate="no">{b.model}</span> · ≈ {formatTokens(contextTotal(b))} / {b.window > 0 ? formatTokens(b.window) : '?'} token
            </p>
            {b.window > 0 && (
              <div className="bar" role="progressbar" aria-label="Bağlam doluluğu" aria-valuenow={Math.min(100, Math.round((contextTotal(b) * 100) / b.window))} aria-valuemin={0} aria-valuemax={100}>
                <span className="bar-fill" style={{ width: `${Math.min(100, (contextTotal(b) * 100) / b.window)}%` }} />
              </div>
            )}
            <HBars
              label="Kategori bazında token"
              items={contextParts(b, warn)
                .filter((p) => p.tokens > 0)
                .sort((x, y) => y.tokens - x.tokens)
                .map((p) => ({ key: p.label, value: p.tokens, text: `≈ ${formatTokens(p.tokens)}${p.pct !== null ? ` · %${p.pct}` : ''}${p.over ? ' · eşik üstü' : ''}` }))}
            />
            {warning && (
              <p className="g7-warn-text" role="status" translate="no">
                {warning}
              </p>
            )}
            {text && (
              <details>
                <summary>CLI çıktısı (kor context)</summary>
                <pre className="g14-pre" translate="no">{text}</pre>
              </details>
            )}
          </>
        )}
      </section>

      <section className="g14-section" aria-label="Zaman içinde" data-kor-cmd="context.history">
        <h3 className="g14-h">Zaman içinde (pencerenin yüzdesi)</h3>
        {history.error ? (
          <p className="error-text">{errorMessage(history.error)}</p>
        ) : !history.data ? (
          <p className="muted">Yükleniyor...</p>
        ) : history.data.length === 0 ? (
          <p className="muted g14-empty">Geçmiş yok.</p>
        ) : (
          <Columns points={contextSeries(history.data)} max={100} label="İstek başına bağlam doluluğu" format={(v) => percentText(v)} />
        )}
      </section>

      <ContextItems sessionId={props.sessionId} />
      <Thresholds key={JSON.stringify(thresholds)} t={thresholds} onSaved={refresh} />
    </div>
  )
}

function Thresholds({ t, onSaved }: { t: ContextThresholds; onSaved: () => void }) {
  const id = useId()
  const [warn, setWarn] = useState(String(t.warn_percent))
  const [auto, setAuto] = useState(String(t.auto_summarize_percent))
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)

  const save = async (e: FormEvent) => {
    e.preventDefault()
    const w = percentSetting(warn)
    const a = percentSetting(auto)
    if (w === null || a === null) return setMessage({ ok: false, text: 'Eşikler 0–100 arası tamsayı olmalı.' })
    try {
      if (Number(w) !== t.warn_percent) await api.core.configSet('context.warn_percent', w)
      if (Number(a) !== t.auto_summarize_percent) await api.core.configSet('context.auto_summarize_percent', a)
      setMessage({ ok: true, text: 'Kaydedildi. Yeni turlarda geçerli.' })
      onSaved()
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    }
  }

  return (
    <form className="g14-section" onSubmit={(e) => void save(e)} aria-labelledby={`${id}-h`} data-kor-cmd="core.configSet">
      <h3 id={`${id}-h`} className="g14-h">Eşikler</h3>
      <div className="g2c-form">
        <label htmlFor={`${id}-warn`}>Kategori uyarısı (pencerenin %'si, 0 = kapalı)</label>
        <input id={`${id}-warn`} className="text-input g9-num" type="number" min={0} max={100} step={1} value={warn} onChange={(e) => setWarn(e.target.value)} />
        <label htmlFor={`${id}-auto`}>Otomatik özetleme (%, 0 = yalnız pencere sonunda)</label>
        <input id={`${id}-auto`} className="text-input g9-num" type="number" min={0} max={100} step={1} value={auto} onChange={(e) => setAuto(e.target.value)} />
      </div>
      <p className="g7-note">
        Özetleyen model: <span className="g7-mono" translate="no">{t.summarize_model || 'sohbet modeli'}</span> · repo haritası bütçesi {formatTokens(t.repo_map_tokens)} token
      </p>
      <div className="g7-inline">
        <button type="submit" className="btn small">Kaydet</button>
        {message && (
          <p className={message.ok ? 'g7-note' : 'error-text'} role="status">
            {message.text}
          </p>
        )}
      </div>
    </form>
  )
}
