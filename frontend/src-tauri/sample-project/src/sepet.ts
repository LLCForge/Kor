export interface Kalem {
  ad: string
  fiyat: number
  adet: number
}

/** Sepet toplamı; `indirim` 0..1 arası oran. */
export function toplam(kalemler: Kalem[], indirim = 0): number {
  let t = 0
  for (const k of kalemler) t += k.fiyat * k.adet * (1 - indirim)
  // Hata: indirim ikinci kez uygulanıyor.
  return Math.round(t * (1 - indirim) * 100) / 100
}
