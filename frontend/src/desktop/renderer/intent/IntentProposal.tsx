// Masaüstü composer'ın niyet kancası (§30, ADR 0056): gönderilen serbest metin önce `intent.route`'a
// gider. Salt okunur komut hemen çalışır, yazan/maliyetli/ağa çıkan komut tam komut satırıyla
// gösterilip onaylanır (onay penceresi arka uçta da açılır, arayüz atlayamaz), belirsizse tek kompakt
// soru sorulur, komut değilse metin ajana gider. Ayar: `intent.enabled`.
import { CalendarClock, Play, Send, Terminal, X } from 'lucide-react'
import { useCallback, useState, type ReactNode } from 'react'
import type { IntentApi, IntentDecision, IntentPlan, IntentRunResult } from '../../ipc-intent.ts'
import { api, errorMessage } from '../api.ts'
import { extra, headline, interval, needsUser, refill, viaLabel } from './format.ts'
import './intent.css'

export const intentApi = (api as unknown as { intent: IntentApi }).intent

interface Pending {
  text: string
  decision: IntentDecision
  deliver: (text: string) => void
  selection?: string
}

function PlanLine({ plan }: { plan: IntentPlan }) {
  const more = extra(plan)
  return (
    <div className="ki-plan">
      <code className="ki-line" translate="no">{plan.line}</code>
      <span className="ki-meta">{plan.name}{more ? ` · ${more}` : ''} · {viaLabel(plan)}</span>
    </div>
  )
}

function Output({ result }: { result: IntentRunResult }) {
  return (
    <figure className="ki-output">
      <figcaption className="ki-meta">
        <Terminal size={12} aria-hidden="true" /> <span translate="no">{result.line}</span>
        {result.code != null && result.code !== 0 ? ` · çıkış kodu ${result.code}` : ''}
      </figcaption>
      <pre translate="no">{result.output.trim() || '(çıktı yok)'}</pre>
    </figure>
  )
}

function Card({ pending, onClose }: { pending: Pending; onClose: () => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [results, setResults] = useState<IntentRunResult[]>([])
  const [values, setValues] = useState<Record<string, string>>({})
  const [decision, setDecision] = useState<IntentDecision>(pending.decision)
  const d = decision

  const runPlans = async (plans: IntentPlan[], agentAfter: string[] = []) => {
    setBusy(true)
    setError(null)
    try {
      for (const p of plans) {
        const r = await intentApi.run(p.command, p.argv)
        setResults((prev) => [...prev, r])
      }
      for (const t of agentAfter) pending.deliver(t)
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const choose = (p: IntentPlan) => {
    // §30.2: seçim yerelde öğrenilir.
    void intentApi.learn(pending.text, p.command).catch(() => undefined)
    if (p.missing.length > 0) setDecision({ kind: 'missing', plan: p })
    else if (p.run === 'auto') void runPlans([p])
    else setDecision({ kind: 'confirm', plan: p })
  }

  const refillRoute = () => {
    setBusy(true)
    intentApi
      .route(refill(pending.text, values), pending.selection ? { selection: pending.selection } : undefined)
      .then((next) => setDecision(next.kind === 'agent' ? d : next))
      .catch((e: unknown) => setError(errorMessage(e)))
      .finally(() => setBusy(false))
  }

  let body: ReactNode = null
  let actions: ReactNode = null
  switch (d.kind) {
    case 'confirm':
      body = <PlanLine plan={d.plan} />
      actions = (
        <button type="button" className="btn primary" disabled={busy} onClick={() => void runPlans([d.plan])} data-kor-cmd="intent.run">
          <Play size={13} aria-hidden="true" /> Çalıştır
        </button>
      )
      break
    case 'run':
      body = <PlanLine plan={d.plan} />
      break
    case 'missing':
      body = (
        <>
          <PlanLine plan={d.plan} />
          <form className="ki-form" onSubmit={(e) => { e.preventDefault(); refillRoute() }}>
            {d.plan.missing.map((m) => (
              <label key={m} className="ki-field">
                <span>{m}</span>
                <input value={values[m] ?? ''} onChange={(e) => setValues({ ...values, [m]: e.target.value })} translate="no" required />
              </label>
            ))}
            <button type="submit" className="btn" disabled={busy} data-kor-cmd="intent.route">Devam</button>
          </form>
        </>
      )
      break
    case 'choose':
      body = (
        <ul className="ki-options" aria-label="Seçenekler">
          {d.options.map((p) => (
            <li key={p.command}>
              <button type="button" className="ki-option" disabled={busy} onClick={() => choose(p)} data-kor-cmd="intent.learn">
                <PlanLine plan={p} />
              </button>
            </li>
          ))}
        </ul>
      )
      break
    case 'pipeline':
      body = (
        <ol className="ki-steps">
          {d.steps.map((s, i) => (
            <li key={i}>{s.type === 'command' ? <PlanLine plan={s.value} /> : <span className="ki-meta">ajan: <span translate="no">{s.value}</span></span>}</li>
          ))}
        </ol>
      )
      actions = (
        <button
          type="button"
          className="btn primary"
          disabled={busy}
          data-kor-cmd="intent.run"
          onClick={() => {
            const plans = d.steps.flatMap((s) => (s.type === 'command' ? [s.value] : []))
            const agent = d.steps.flatMap((s) => (s.type === 'agent' ? [s.value] : []))
            void runPlans(plans, agent)
          }}
        >
          <Play size={13} aria-hidden="true" /> Sırayla çalıştır
        </button>
      )
      break
    case 'schedule':
      body = (
        <div className="ki-plan">
          <span translate="no">{d.prompt}</span>
          <span className="ki-meta">{interval(d.interval_minutes)} · ilk çalışma {new Date(d.first_run).toLocaleString()}</span>
        </div>
      )
      actions = (
        <button
          type="button"
          className="btn primary"
          disabled={busy}
          data-kor-cmd="intent.schedule"
          onClick={() => {
            setBusy(true)
            intentApi
              .schedule(d.prompt, d.interval_minutes, d.first_run)
              .then(() => onClose())
              .catch((e: unknown) => setError(errorMessage(e)))
              .finally(() => setBusy(false))
          }}
        >
          <CalendarClock size={13} aria-hidden="true" /> Rutini oluştur
        </button>
      )
      break
    case 'agent':
      break
  }

  return (
    <section className="ki-card" role="status" aria-live="polite" aria-label="Komut önerisi" data-kor-cmd="intent.route">
      <header className="ki-head">
        <strong>{headline(d)}</strong>
        <button type="button" className="action-btn" onClick={onClose} aria-label="Öneriyi kapat" title="Kapat">
          <X size={13} aria-hidden="true" />
        </button>
      </header>
      {body}
      {results.map((r, i) => <Output key={i} result={r} />)}
      {error && <p className="ki-error" role="alert">{error}</p>}
      <footer className="ki-actions">
        {actions}
        <button
          type="button"
          className="btn"
          disabled={busy}
          onClick={() => {
            onClose()
            pending.deliver(pending.text)
          }}
          title="Komut çalıştırmadan metni ajana gönder"
        >
          <Send size={13} aria-hidden="true" /> Ajana gönder
        </button>
      </footer>
    </section>
  )
}

/**
 * Composer kancası: `gate(text, deliver)` metni üstlenirse `true` döner (yönlendirme sürüyor);
 * komut değilse `deliver(text)` çağrılır. `element` composer'ın üstünde gösterilir.
 */
export function useIntentGate(): { gate: (text: string, deliver: (t: string) => void, selection?: string) => boolean; element: ReactNode } {
  const [pending, setPending] = useState<Pending | null>(null)
  const [auto, setAuto] = useState<{ plan: IntentPlan; result: IntentRunResult | null; error: string | null } | null>(null)

  const gate = useCallback((text: string, deliver: (t: string) => void, selection?: string) => {
    if (text.trim() === '' || text.length > 4000) return false
    setAuto(null)
    intentApi
      .route(text, selection ? { selection } : undefined)
      .then((d) => {
        if (d.kind === 'agent') deliver(text)
        else if (d.kind === 'run') {
          setAuto({ plan: d.plan, result: null, error: null })
          intentApi
            .run(d.plan.command, d.plan.argv)
            .then((r) => setAuto({ plan: d.plan, result: r, error: null }))
            .catch((e: unknown) => setAuto({ plan: d.plan, result: null, error: errorMessage(e) }))
        } else if (needsUser(d)) setPending({ text, decision: d, deliver, selection })
      })
      // Yönlendirme başarısızsa metin kaybolmaz: ajana gider.
      .catch(() => deliver(text))
    return true
  }, [])

  const element = (
    <>
      {pending && <Card key={pending.text} pending={pending} onClose={() => setPending(null)} />}
      {auto && (
        <section className="ki-card" role="status" aria-live="polite" aria-label="Komut çıktısı" data-kor-cmd="intent.run">
          <header className="ki-head">
            <strong>{headline({ kind: 'run', plan: auto.plan })}</strong>
            <button type="button" className="action-btn" onClick={() => setAuto(null)} aria-label="Çıktıyı kapat" title="Kapat">
              <X size={13} aria-hidden="true" />
            </button>
          </header>
          <PlanLine plan={auto.plan} />
          {auto.result ? <Output result={auto.result} /> : !auto.error && <p className="ki-meta">Çalışıyor…</p>}
          {auto.error && <p className="ki-error" role="alert">{auto.error}</p>}
        </section>
      )}
    </>
  )
  return { gate, element }
}
