// Sandbox kartı: saf yardımcılar (test: sandbox-shape.test.ts). Kurallar kor_sandbox::valid_allow ile aynı.

/** `host:port` (port sayı ya da `*`) veya yalnızca `*`. */
export function validAllow(entry: string): boolean {
  if (entry === '*') return true
  const at = entry.lastIndexOf(':')
  if (at <= 0) return false
  const host = entry.slice(0, at)
  const port = entry.slice(at + 1)
  if (host.length > 253 || !/^[A-Za-z0-9.\-*[\]:]+$/.test(host)) return false
  if (port === '*') return true
  if (!/^\d{1,5}$/.test(port)) return false
  const n = Number(port)
  return n > 0 && n <= 65535
}

/** Satır ya da virgülle ayrılmış izin listesi; boşlar atılır, geçersizler ayrı döner. */
export function parseAllowlist(text: string): { entries: string[]; invalid: string[] } {
  const items = text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean)
  return {
    entries: [...new Set(items.filter(validAllow))],
    invalid: items.filter((s) => !validAllow(s)),
  }
}

/** Bellek (MiB) ve CPU (%) alanları: tam sayı, sınırlar içinde; 0 = sınır yok. */
export function parseLimit(text: string, max: number): number | null {
  const t = text.trim()
  if (!/^\d+$/.test(t)) return null
  const n = Number(t)
  return n <= max ? n : null
}

export function limitText(value: number, unit: string, enforced: boolean, isPercent = false): string {
  if (value === 0 || (isPercent && value >= 100)) return 'sınır yok'
  return `${value}${unit} · ${enforced ? 'uygulanıyor' : 'bu arka uç uygulayamıyor (yalnız zaman aşımı)'}`
}
