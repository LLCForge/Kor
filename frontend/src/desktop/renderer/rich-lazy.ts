// [temel] Ağır/isteğe bağlı kütüphaneler ayrı IIFE paketi (dist/renderer/<ad>.js) olarak derlenir ve ilk kez
//         gerektiğinde bir <script> ile tembel yüklenir. Paket, API'sini window[globalKey] üzerinden sunar
//         (bkz. mermaid-entry.ts). build-desktop.ts içindeki lazyBundles listesi bu paketleri üretir.
const pending = new Map<string, Promise<unknown>>()

/**
 * Bir tembel paketi yükler ve window[globalKey] üzerinden sunduğu API'yi döndürür. Aynı kaynak için
 * yükleme bir kez yapılır; hata halinde yeniden denenebilmesi için önbellek temizlenir.
 */
export function loadRichBundle<T>(src: string, globalKey: string): Promise<T> {
  const win = window as unknown as Record<string, unknown>
  const existing = win[globalKey]
  if (existing) return Promise.resolve(existing as T)
  let promise = pending.get(src)
  if (!promise) {
    promise = new Promise<unknown>((resolve, reject) => {
      const script = document.createElement('script')
      script.src = src
      script.onload = () => {
        const api = win[globalKey]
        if (api) {
          resolve(api)
        } else {
          pending.delete(src)
          reject(new Error(`${globalKey} başlatılamadı`))
        }
      }
      script.onerror = () => {
        pending.delete(src)
        reject(new Error(`${src} yüklenemedi`))
      }
      document.head.append(script)
    })
    pending.set(src, promise)
  }
  return promise as Promise<T>
}
