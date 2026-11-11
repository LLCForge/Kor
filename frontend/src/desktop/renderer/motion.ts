// Hareket politikası (§16.2 kural 3, §23.1, §32.3): kökteki `data-motion` (Ayarlar > Görünüm > Animasyonlar),
// işletim sisteminin "hareketi azalt" isteği, `ui.reduced_motion`/ekran okuyucu (`kor-ui-still`) ve düşük güç modu
// (`data-low-power`) tek yerde birleşir. CSS aynı kuralları `:root[data-motion]` seçicileriyle uygular; JS
// (kor renk döngüsü, SMIL <img> yedekleri, açılış/kapanış süreleri) buradan okur.
import { useSyncExternalStore } from 'react'

export type MotionPref = 'system' | 'on' | 'off'
export const MOTION_PREFS: readonly MotionPref[] = ['system', 'on', 'off']
const KEY = 'kor.motion'
const OS_QUERY = '(prefers-reduced-motion: reduce)'

export interface MotionInputs {
  pref: MotionPref
  osReduce: boolean
  lowPower: boolean
  still: boolean
}

/** Hareket kapalı mı. "Açık" yalnızca işletim sistemi isteğini geçersiz kılar; düşük güç ve okunabilirlik ayarı kalır. */
export function isStill({ pref, osReduce, lowPower, still }: MotionInputs): boolean {
  if (lowPower || still || pref === 'off') return true
  return pref === 'system' && osReduce
}

function root(): HTMLElement | null {
  return typeof document === 'undefined' ? null : document.documentElement
}

export function readMotionPref(): MotionPref {
  try {
    const v = window.localStorage.getItem(KEY)
    return MOTION_PREFS.find((p) => p === v) ?? 'system'
  } catch {
    return 'system'
  }
}

/** Kök öğeye uygular ve hatırlar (depolama kapalıysa bu oturumla sınırlı). */
export function setMotionPref(pref: MotionPref): void {
  const el = root()
  if (el) el.dataset.motion = pref
  try {
    window.localStorage.setItem(KEY, pref)
  } catch {
    // Depolama kapalı: tercih bu oturumla sınırlı kalır.
  }
}

/** React çizmeden önce bir kez (main.tsx): ilk kare doğru hareket kipinde çizilir. */
export function bootMotion(): void {
  const el = root()
  if (el) el.dataset.motion = readMotionPref()
}

export function motionInputs(): MotionInputs {
  const el = root()
  const pref = MOTION_PREFS.find((p) => p === el?.dataset.motion) ?? 'system'
  return {
    pref,
    osReduce: typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia(OS_QUERY).matches,
    lowPower: el?.dataset.lowPower === '1',
    still: el?.classList.contains('kor-ui-still') ?? false,
  }
}

export const motionStill = (): boolean => isStill(motionInputs())

/** Açılış/kapanış süresi (ms): hareket kapalıysa 0, değilse tema token'ı (`--kor-dur-menu` / `--kor-dur-panel`). */
export function motionMs(kind: 'hover' | 'menu' | 'panel'): number {
  const el = root()
  if (!el || motionStill()) return 0
  const v = parseFloat(getComputedStyle(el).getPropertyValue(`--kor-dur-${kind}`))
  return Number.isFinite(v) ? v : 0
}

function subscribe(onChange: () => void): () => void {
  const el = root()
  if (!el) return () => {}
  const mq = window.matchMedia(OS_QUERY)
  const obs = new MutationObserver(onChange)
  obs.observe(el, { attributes: true, attributeFilter: ['class', 'data-motion', 'data-low-power'] })
  mq.addEventListener('change', onChange)
  return () => {
    obs.disconnect()
    mq.removeEventListener('change', onChange)
  }
}

const key = (i: MotionInputs) => `${i.pref}|${+i.osReduce}|${+i.lowPower}|${+i.still}`
const fromKey = (k: string): MotionInputs => {
  const [pref, os, low, still] = k.split('|')
  return { pref: pref as MotionPref, osReduce: os === '1', lowPower: low === '1', still: still === '1' }
}

/** Hareket girdileri (Ayarlar notu için); kök sınıfı, `data-*` ya da OS ayarı değişince yeniden çizer. */
export function useMotionInputs(): MotionInputs {
  return fromKey(useSyncExternalStore(subscribe, () => key(motionInputs()), () => 'system|0|0|0'))
}

export const useMotionStill = (): boolean => isStill(useMotionInputs())

/** Giriş animasyonları yalnızca ilk açılışta (§23.1): kısa süre sonra `data-entered` konur, CSS sonrakileri kapatır. */
export function markEntered(afterMs = 2500): void {
  window.setTimeout(() => {
    const el = root()
    if (el) el.dataset.entered = '1'
  }, afterMs)
}
