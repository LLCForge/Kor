import { useCallback, useState } from 'react'

// Yalnızca arayüz tercihleri (diff görünümü, .gitignore anahtarı vb.) için; veri değil.
// Depolama kapalı/bozuksa tercih bu oturumla sınırlı kalır.

function read<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const value = window.localStorage.getItem(key)
    return allowed.find((a) => a === value) ?? fallback
  } catch (err) {
    console.warn('Tercih okunamadı:', key, err)
    return fallback
  }
}

/** Değeri izin verilen seçeneklerle sınırlı, localStorage'da hatırlanan durum. */
export function usePersistentChoice<T extends string>(key: string, allowed: readonly T[], fallback: T): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => read(key, allowed, fallback))
  const update = useCallback(
    (next: T) => {
      setValue(next)
      try {
        window.localStorage.setItem(key, next)
      } catch (err) {
        console.warn('Tercih kaydedilemedi:', key, err)
      }
    },
    [key],
  )
  return [value, update]
}
