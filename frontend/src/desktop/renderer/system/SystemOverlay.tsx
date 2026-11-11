// §19.3 sağ alt sağlık kartları ve §19.1 "Kor masaüstünü kontrol ediyor" göstergesi (ADR 0059).
// Kartlar `kor:system/health` ile gelir (aynı uyarı bildirim merkezine de yazılır), kapatılabilir ve
// eylemlidir: "Temizle" Sistem görünümünde onaylı temizliği açar, "Sessize al" o türü kapatır.
// Gösterge her masaüstü eyleminde (`kor:computer/activity`) görünür, 5 sn sessizlikte kaybolur;
// "Durdur" Kor'u duraklatır (sonraki adım onay ister).
import { BellOff, Brush, Square, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { ComputerActivity, HealthAlert } from '../../ipc-system.ts'
import { EmberPulse } from '../Logo.tsx'
import { errorMessage } from '../api.ts'
import { motionMs } from '../motion.ts'
import '../panels/system/panel.css'
import { CLEAN_EVENT, computerApi, systemApi } from '../panels/system/bridge.ts'
import { controlling, pushCard } from '../panels/system/shape.ts'

export function SystemOverlay({ openSystem }: { openSystem: () => void }) {
  const [cards, setCards] = useState<HealthAlert[]>([])
  const [activity, setActivity] = useState<ComputerActivity | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [problem, setProblem] = useState<string | null>(null)
  // Kapatılan kartlar çıkış animasyonu (`--kor-dur-panel`) bitene kadar `leaving` sınıfıyla kalır.
  const [leaving, setLeaving] = useState<ReadonlySet<string>>(new Set())

  useEffect(() => systemApi.onHealth((a) => setCards((c) => pushCard(c, a))), [])
  useEffect(() => computerApi.onActivity((a) => { setActivity(a); setNow(Date.now()) }), [])
  // Gösterge, son eylemden 5 sn sonra kendiliğinden kapanır (sahte sayaç değil: tek zamanlayıcı).
  useEffect(() => {
    if (!activity?.active) return
    const t = window.setTimeout(() => setNow(Date.now()), 5100)
    return () => window.clearTimeout(t)
  }, [activity])

  const dismiss = (id: string) => {
    const drop = () => {
      setCards((c) => c.filter((x) => x.id !== id))
      setLeaving((l) => new Set([...l].filter((x) => x !== id)))
    }
    const ms = motionMs('panel')
    if (ms === 0) return drop()
    setLeaving((l) => new Set(l).add(id))
    window.setTimeout(drop, ms)
  }
  const clean = (id: string) => {
    dismiss(id)
    openSystem()
    requestAnimationFrame(() => window.dispatchEvent(new Event(CLEAN_EVENT)))
  }
  const mute = async (a: HealthAlert) => {
    try {
      await systemApi.mute(a.kind)
      setCards((c) => c.filter((x) => x.kind !== a.kind))
    } catch (err) {
      setProblem(errorMessage(err))
    }
  }
  const stop = async () => {
    try {
      setActivity(await computerApi.stop().then((s) => s.activity))
    } catch (err) {
      setProblem(errorMessage(err))
    }
  }

  const on = controlling(activity, now)
  const paused = activity?.paused ?? null
  return (
    <>
      {(on || paused) && (
        <div className={`ksys-control${paused ? ' paused' : ''}`} role="status" aria-live="polite" data-kor-event="kor:computer/activity">
          {on && <EmberPulse size={16} label="Kor çalışıyor" />}
          <span translate="no">
            {paused
              ? `Kor durdu: kontrolü siz aldınız (${paused === 'mouse' ? 'fare' : 'klavye'}). Sonraki adım onayınızı isteyecek.`
              : `Kor masaüstünü kontrol ediyor${activity?.app ? ` · ${activity.app}` : ''}. Durdurmak için fareyi oynatın.`}
          </span>
          {on && (
            <button type="button" className="btn small" onClick={() => void stop()} data-kor-cmd="computer.stop">
              <Square size={14} aria-hidden="true" />Durdur
            </button>
          )}
          {paused && (
            <button type="button" className="action-btn" aria-label="Göstergeyi kapat" title="Kapat" onClick={() => setActivity(null)}>
              <X size={14} aria-hidden="true" />
            </button>
          )}
        </div>
      )}
      {cards.length > 0 && (
        <section className="ksys-toasts" aria-label="Sağlık uyarıları" data-kor-event="kor:system/health">
          {cards.map((a) => (
            <article key={a.id} className={`ksys-toast ${a.level}${leaving.has(a.id) ? ' leaving' : ''}`} inert={leaving.has(a.id)} role={a.level === 'critical' ? 'alert' : 'status'}>
              <div className="ksys-toast-head">
                <EmberPulse size={14} label={a.level === 'critical' ? 'Kritik' : 'Uyarı'} />
                <strong>{a.title}</strong>
                <button type="button" className="action-btn" aria-label="Uyarıyı kapat" title="Kapat" onClick={() => dismiss(a.id)}>
                  <X size={14} aria-hidden="true" />
                </button>
              </div>
              {a.body && <p className="ksys-toast-body" translate="no">{a.body}</p>}
              <div className="ksys-toast-actions">
                <button type="button" className="action-btn" aria-label="Bu uyarı türünü sessize al" title="Sessize al" onClick={() => void mute(a)} data-kor-cmd="system.mute">
                  <BellOff size={14} aria-hidden="true" />
                </button>
                {a.clean && (
                  <button type="button" className="btn small primary" onClick={() => clean(a.id)} data-kor-cmd="system.cleanSuggest">
                    <Brush size={14} aria-hidden="true" />Temizle
                  </button>
                )}
              </div>
            </article>
          ))}
          {problem && <p className="error-text" role="alert">{problem}</p>}
        </section>
      )}
    </>
  )
}
