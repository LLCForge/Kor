// Kor'un asla yazmadığı alanlar (§18.2, §27.12, ADR 0058): parola, kart numarası, CVV/CVC, son kullanma,
// tek kullanımlık kod, PIN, IBAN. Çekirdekteki `browser_assist::forms::sensitive` ile aynı kelime listesi; sayfa
// betiği (page/index.ts) her yazmadan önce bunu çağırır, çekirdeğin kararına güvenmez.

export interface FieldDesc {
  type: string
  autocomplete: string
  label: string
  name: string
  id: string
  placeholder: string
}

const PHRASES = [
  'password', 'passwd', 'parola', 'şifre', 'sifre', 'mot de passe', 'card number', 'cardnumber',
  'kart no', 'kart numarası', 'kredi kartı', 'numéro de carte', 'cvv', 'cvc', 'cvn', 'cvv2',
  'security code', 'güvenlik kodu', 'cryptogramme', 'expiry', 'expiration', 'son kullanma',
  'one time', 'otp', 'doğrulama kodu', 'verification code', 'code de vérification', 'pin',
  'iban', 'swift',
]

export function words(s: string): string[] {
  // Aksanlar atılır (İ→i, Ş→s): "PIN", "ŞİFRE" ve "sifre" aynı sözcüklere iner; ifade listesi de aynı yoldan geçer.
  return s
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w.length > 0)
}

/** Sözcük sınırıyla öbek eşleşmesi: `pin` "PIN kodu"nu bulur, "shipping"i bulmaz. */
export function hasPhrase(hay: string[], phrase: string): boolean {
  const p = words(phrase)
  if (p.length === 0) return false
  for (let i = 0; i + p.length <= hay.length; i++) {
    if (p.every((w, j) => hay[i + j] === w)) return true
  }
  return false
}

export function sensitive(f: FieldDesc): boolean {
  const type = f.type.toLowerCase()
  if (type === 'password' || type === 'hidden' || type === 'file') return true
  const ac = f.autocomplete.toLowerCase().split(/\s+/)
  if (ac.some((t) => t.startsWith('cc-') || ['one-time-code', 'current-password', 'new-password', 'transaction-amount'].includes(t))) return true
  const hay = words([f.label, f.name, f.id, f.placeholder].join(' ').replace(/[-_]/g, ' '))
  return PHRASES.some((p) => hasPhrase(hay, p))
}
