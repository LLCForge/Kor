// [g30] Sır sızıntı taraması: metinde API anahtarı, token, özel anahtar gibi sır BENZERİ dizeleri bulur.
// Amaç önleme: commit mesajı, model çıktısı, otomatik hafıza adayı gibi yerlerde sır görünürse işaretlenir/maskeleir.
// Değer deposu gerektirmez; desen + basit entropi ile çalışır. secrets.ts maskSecrets uygulamanın KENDİ bilinen
// anahtarlarını maskeler; bu ise bilinmeyen sır benzeri dizeleri yakalar.

export interface SecretHit {
  /** Bulgu türü (ör. "OpenAI/Venice anahtarı", "AWS erişim anahtarı", "özel anahtar"). */
  kind: string
  /** Eşleşen dizenin maskeli hali (ilk 4 + son 2 karakter görünür). */
  masked: string
}

interface Pattern {
  kind: string
  re: RegExp
}

// Yüksek güvenli, az yanlış-pozitifli desenler. Her biri global; tüm eşleşmeler toplanır.
const PATTERNS: readonly Pattern[] = [
  { kind: 'OpenAI/Venice anahtarı', re: /\bsk-[A-Za-z0-9_-]{20,}\b/g },
  { kind: 'Anthropic anahtarı', re: /\bsk-ant-[A-Za-z0-9_-]{20,}\b/g },
  { kind: 'AWS erişim anahtarı', re: /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/g },
  { kind: 'GitHub token', re: /\bgh[pousr]_[A-Za-z0-9]{36,}\b/g },
  { kind: 'Google API anahtarı', re: /\bAIza[A-Za-z0-9_-]{35}\b/g },
  { kind: 'Slack token', re: /\bxox[baprs]-[A-Za-z0-9-]{10,}\b/g },
  { kind: 'JWT', re: /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/g },
  { kind: 'özel anahtar bloğu', re: /-----BEGIN (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----/g },
  // Genel: "api_key = 'uzun-dize'" / "token: \"...\"" gibi atamalar (yüksek entropili değer).
  { kind: 'gömülü sır', re: /\b(?:api[_-]?key|secret|token|password|passwd|access[_-]?key)\b["'\s:=]{1,4}["']?([A-Za-z0-9/_+=-]{16,})["']?/gi },
]

/** Maskele: ilk 4 + son 2 görünür, ortası •. Kısa dizelerde tamamı • olur. */
export function maskValue(value: string): string {
  if (value.length <= 8) return '•'.repeat(value.length)
  return `${value.slice(0, 4)}${'•'.repeat(Math.min(12, value.length - 6))}${value.slice(-2)}`
}

/** Metindeki sır benzeri dizeleri döndürür (tekilleştirilmiş). Boşsa metin temiz. */
export function scanSecrets(text: string): SecretHit[] {
  if (!text) return []
  const hits = new Map<string, SecretHit>()
  for (const { kind, re } of PATTERNS) {
    for (const m of text.matchAll(re)) {
      // Atama deseninde yakalanan grup (değer), diğerlerinde tüm eşleşme.
      const value = m[1] ?? m[0]
      if (!value || value.length < 8) continue
      if (hits.has(value)) continue
      hits.set(value, { kind, masked: maskValue(value) })
    }
  }
  return [...hits.values()]
}

/** Metinde sır var mı (hızlı kontrol). */
export function hasSecret(text: string): boolean {
  return scanSecrets(text).length > 0
}
