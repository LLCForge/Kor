// [g8] Etkileşimli artifact durumunu (yapılacaklar, quiz, sekme vb.) yalnızca o cihazda saklayan kanca (#119).
//      Depolama kapalı/dolu/bozuk olabilir: her erişim try/catch'lidir ve başlangıç değerine düşer.
//      Paylaşılmaz, sunucuya gitmez, başka oturumda boş gelebilir.
import { useCallback, useState } from 'react'

const PREFIX = 'g8:'

function load<T>(key: string, initial: T): T {
  try {
    const raw = window.localStorage.getItem(PREFIX + key)
    if (raw === null) return initial
    return JSON.parse(raw) as T
  } catch (err) {
    console.warn('Yerel durum okunamadı:', key, err)
    return initial
  }
}

/** localStorage'da JSON olarak hatırlanan bileşen durumu. Depolama yoksa yalnızca bu oturumda yaşar. */
export function usePersistentState<T>(key: string, initial: T): [T, (next: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => load(key, initial))
  const update = useCallback(
    (next: T | ((prev: T) => T)) => {
      setValue((prev) => {
        const resolved = typeof next === 'function' ? (next as (p: T) => T)(prev) : next
        try {
          window.localStorage.setItem(PREFIX + key, JSON.stringify(resolved))
        } catch (err) {
          console.warn('Yerel durum kaydedilemedi:', key, err)
        }
        return resolved
      })
    },
    [key],
  )
  return [value, update]
}
