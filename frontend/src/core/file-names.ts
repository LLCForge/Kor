// Dosya ağacından gelen yeni ad doğrulaması. Tüm platformlarda Windows kuralları uygulanır: depo başka işletim sisteminde de açılabilsin.
// Node'a bağımlı değildir: arayüz de anlık doğrulama için kullanır, ana süreç her işlemde yeniden doğrular.

const RESERVED = /^(con|prn|aux|nul|com[0-9¹²³]|lpt[0-9¹²³])(\..*)?$/i
const INVALID_CHARS = /[<>:"|?*\u0000-\u001f]/
const MAX_NAME_LENGTH = 255

/** Geçerliyse null, değilse kullanıcıya gösterilecek Türkçe hata. */
export function validateFileName(name: string): string | null {
  if (name.trim() === '') return 'Ad boş olamaz.'
  if (name === '.' || name === '..') return 'Bu ad kullanılamaz.'
  if (name.includes('/') || name.includes('\\')) return 'Ad yol ayırıcı (/ veya \\) içeremez.'
  if (INVALID_CHARS.test(name)) return 'Ad şu karakterleri içeremez: < > : " | ? * ve kontrol karakterleri.'
  if (/[. ]$/.test(name)) return 'Ad nokta veya boşlukla bitemez.'
  if (name !== name.trimStart()) return 'Ad boşlukla başlayamaz.'
  if (RESERVED.test(name)) return `"${name}" işletim sistemi için ayrılmış bir ad.`
  if (new TextEncoder().encode(name).length > MAX_NAME_LENGTH) return 'Ad çok uzun (en fazla 255 bayt).'
  return null
}
