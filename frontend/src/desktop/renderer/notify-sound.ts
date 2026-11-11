// [cdx] Bildirim sesleri: assets/sounds/*.wav (CLI/TUI ve işletim sistemi bildirimleriyle aynı dosyalar, §28.4).
// Dosya çalınamazsa aynı tonlar Web Audio ile üretilir.
// 'soft' tek yumuşak ton, 'double' çift bip, 'bell' sönümlenen zil, 'chime' yükselen üç nota.
import type { NotificationSound } from '../g7-types.ts'
import bellUrl from 'kor-assets/sounds/bell.wav'
import chimeUrl from 'kor-assets/sounds/chime.wav'
import doubleUrl from 'kor-assets/sounds/double.wav'
import softUrl from 'kor-assets/sounds/soft.wav'

const FILES: Record<NotificationSound, string> = { soft: softUrl, double: doubleUrl, bell: bellUrl, chime: chimeUrl }

export const SOUND_OPTIONS: ReadonlyArray<{ id: NotificationSound; label: string }> = [
  { id: 'soft', label: 'Yumuşak ton' },
  { id: 'double', label: 'Çift bip' },
  { id: 'bell', label: 'Zil' },
  { id: 'chime', label: 'Üç nota' },
]

let ctx: AudioContext | null = null
function audio(): AudioContext | null {
  try {
    ctx ??= new AudioContext()
    if (ctx.state === 'suspended') void ctx.resume()
    return ctx
  } catch {
    return null
  }
}

function tone(ac: AudioContext, freq: number, at: number, dur: number, gain = 0.12, type: OscillatorType = 'sine'): void {
  const osc = ac.createOscillator()
  const g = ac.createGain()
  osc.type = type
  osc.frequency.setValueAtTime(freq, at)
  g.gain.setValueAtTime(0, at)
  g.gain.linearRampToValueAtTime(gain, at + 0.01)
  g.gain.exponentialRampToValueAtTime(0.0001, at + dur)
  osc.connect(g).connect(ac.destination)
  osc.start(at)
  osc.stop(at + dur + 0.02)
}

/** Seçili sesi çalar (dosyadan; olmazsa sentezle); ses üretilemezse sessizce geçer. */
export function playNotificationSound(name: NotificationSound): void {
  try {
    const a = new Audio(FILES[name])
    a.volume = 0.8
    void a.play().catch(() => synth(name))
  } catch {
    synth(name)
  }
}

function synth(name: NotificationSound): void {
  const ac = audio()
  if (!ac) return
  const t = ac.currentTime
  switch (name) {
    case 'soft':
      tone(ac, 660, t, 0.35)
      break
    case 'double':
      tone(ac, 880, t, 0.12, 0.1, 'square')
      tone(ac, 880, t + 0.18, 0.12, 0.1, 'square')
      break
    case 'bell':
      tone(ac, 1046, t, 1.2, 0.14, 'triangle')
      tone(ac, 1568, t, 0.8, 0.05, 'triangle')
      break
    case 'chime':
      tone(ac, 523, t, 0.3)
      tone(ac, 659, t + 0.16, 0.3)
      tone(ac, 784, t + 0.32, 0.45)
      break
  }
}
