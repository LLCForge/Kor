// Sayı, tarih ve süre biçimleri arayüz diline göre: "1.234,5" / "1,234.5", "8dk önce" / "8 min ago".
// Türkçede eldeki yardımcılar kendi biçimini korur (testler ve ekran görüntüleri değişmez); diğer
// dillerde bu dosyadaki `Intl` biçimleri kullanılır.
import { getUiLanguage, type UiLang } from './i18n/translate.ts'

const LOCALES: Record<UiLang, string> = { tr: 'tr-TR', en: 'en-US', fr: 'fr-FR' }

/** `Intl` / `toLocaleString` için etkin arayüz dilinin yerel ayarı. */
export function uiLocale(): string {
  return LOCALES[getUiLanguage()] ?? 'tr-TR'
}

/** Türkçe dışındaki dillerde süre ("9s", "1m 05s", "1h 02m"; fr: "9 s", "1 min 05 s"); Türkçede `null`. */
export function durationIntl(totalSeconds: number): string | null {
  const lang = getUiLanguage()
  if (lang === 'tr') return null
  const t = Math.max(0, Math.floor(totalSeconds))
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const s = t % 60
  const p = (n: number) => String(n).padStart(2, '0')
  if (lang === 'fr') return h > 0 ? `${h} h ${p(m)}` : m > 0 ? `${m} min ${p(s)} s` : `${s} s`
  return h > 0 ? `${h}h ${p(m)}m` : m > 0 ? `${m}m ${p(s)}s` : `${s}s`
}

/** Türkçe dışındaki dillerde `seconds` önce ("5 min ago", "il y a 5 min", "just now"); Türkçede `null`. */
export function agoIntl(seconds: number): string | null {
  if (getUiLanguage() === 'tr') return null
  const s = Math.max(0, Math.round(seconds))
  const rtf = new Intl.RelativeTimeFormat(uiLocale(), { style: 'short', numeric: 'auto' })
  if (s < 45) return rtf.format(0, 'second')
  const units: Array<[Intl.RelativeTimeFormatUnit, number]> = [
    ['minute', 60],
    ['hour', 3600],
    ['day', 86_400],
    ['month', 30 * 86_400],
    ['year', 365 * 86_400],
  ]
  let pick = units[0]!
  for (const u of units) if (s >= u[1]) pick = u
  return rtf.format(-Math.max(1, Math.round(s / pick[1])), pick[0])
}

/** Tek başına yüzde: "%42" (tr), "42%" (en), "42 %" (fr). Cümle içindeki yüzdeler çeviri anahtarında kalır. */
export function percentText(value: number | string): string {
  switch (getUiLanguage()) {
    case 'en':
      return `${value}%`
    case 'fr':
      return `${value} %`
    default:
      return `%${value}`
  }
}
