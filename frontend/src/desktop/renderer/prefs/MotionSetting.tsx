// Ayarlar > Görünüm > Animasyonlar (§16.2 kural 3, §23.1): Sistem / Açık / Kapalı ve hareketin neden durduğunu
// söyleyen not. Tercih bu cihazda saklanır (motion.ts, localStorage; uiScale gibi); kök öğede `data-motion` olur.
import { useId, useState } from 'react'
import { readMotionPref, setMotionPref, useMotionInputs, type MotionInputs, type MotionPref } from '../motion.ts'

const OPTIONS: ReadonlyArray<{ id: MotionPref; label: string }> = [
  { id: 'system', label: 'Sistem' },
  { id: 'on', label: 'Her zaman açık' },
  { id: 'off', label: 'Kapalı' },
]

/** Hareket neden kapalı: kullanıcıya gösterilecek nedenler (boşsa animasyonlar oynuyor). */
export function stillReasons(m: MotionInputs): string[] {
  const out: string[] = []
  if (m.pref === 'system' && m.osReduce) {
    out.push('İşletim sistemi hareketi azaltmayı istiyor (Windows: Erişilebilirlik > Görsel efektler > Animasyon efektleri kapalı). Animasyonları yine de görmek için "Her zaman açık"ı seç.')
  }
  if (m.lowPower) out.push('Düşük güç modu açık (perf.low_power): pil ya da işlemciyi korumak için animasyonlar kapalı.')
  if (m.still) out.push('Okunabilirlik > Azaltılmış hareket ya da Ekran okuyucu modu açık; bu ayar animasyonları her zaman durdurur.')
  return out
}

export function MotionSetting() {
  const id = useId()
  const [pref, setPref] = useState<MotionPref>(readMotionPref)
  const inputs = useMotionInputs()
  const reasons = stillReasons(inputs)
  const choose = (next: MotionPref) => {
    setMotionPref(next)
    setPref(next)
  }
  return (
    <fieldset className="g7-fieldset g7-lang-fieldset">
      <legend className="g7-setting-title">Animasyonlar</legend>
      <p className="g7-note">Sistem: işletim sisteminin "hareketi azalt" ayarına uyar. Her zaman açık: işletim sistemi ne derse desin oynatır. Kapalı: arayüz durağan kalır.</p>
      <div className="g7-lang-options" role="radiogroup" aria-label="Animasyonlar">
        {OPTIONS.map((o) => (
          <label key={o.id} className={`g7-lang${pref === o.id ? ' active' : ''}`} htmlFor={`${id}-${o.id}`}>
            <input id={`${id}-${o.id}`} type="radio" name={`${id}-motion`} checked={pref === o.id} onChange={() => choose(o.id)} />
            <span className="g7-setting-title">{o.label}</span>
          </label>
        ))}
      </div>
      {reasons.length > 0 && (
        <div className="kor-motion-note" role="status">
          <strong>Animasyonlar şu an kapalı.</strong>
          {reasons.map((r) => (
            <p key={r}>{r}</p>
          ))}
        </div>
      )}
    </fieldset>
  )
}
