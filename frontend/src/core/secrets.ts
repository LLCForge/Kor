const MASK = '[maskelendi]'
const MIN_SECRET_CHARS = 8

// Sıra önemli: JWT, "Bearer <jwt>" kuralından önce maskelenir.
const PATTERNS: ReadonlyArray<readonly [RegExp, string]> = [
  [/-----BEGIN ([A-Z0-9 ]*)PRIVATE KEY-----[\s\S]*?(?:-----END \1PRIVATE KEY-----|$)/g, `${MASK} (özel anahtar)`],
  [/\beyJ[A-Za-z0-9_-]{5,}\.eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*/g, `${MASK} (JWT)`],
  // Rakam şartı, "sk-loading-spinner" gibi sıradan adları eler.
  [/\bsk-(?=[A-Za-z0-9_-]*\d)[A-Za-z0-9_-]{20,}/g, `sk-${MASK}`],
  [/\b(AKIA|ASIA)[0-9A-Z]{16}\b/g, `$1${MASK}`],
  [/\b(gh[pousr]_)[A-Za-z0-9]{30,}/g, `$1${MASK}`],
  [/\bgithub_pat_[A-Za-z0-9_]{20,}/g, `github_pat_${MASK}`],
  [/\b(xox[abposr]-)[A-Za-z0-9-]{10,}/g, `$1${MASK}`],
  [/\b(Bearer\s+)(?=[A-Za-z0-9._~+/=-]*\d)[A-Za-z0-9._~+/=-]{16,}/gi, `$1${MASK}`],
]

const secretProviders: Array<() => string | null | undefined> = [() => process.env.VENICE_API_KEY]
const registered = new Set<string>()

/** Desen dışı gizli değerler (ör. masaüstünde şifreli saklanan API anahtarı) için kaynak ekler. */
export function addSecretProvider(provider: () => string | null | undefined): void {
  secretProviders.push(provider)
}

/** Oturum istemcisinin anahtarı gibi sabit değerler; aynı değer tekrar eklenmez. */
export function registerSecret(value: unknown): void {
  if (typeof value === 'string' && value.trim().length >= MIN_SECRET_CHARS) registered.add(value.trim())
}

function knownSecrets(): string[] {
  const values = new Set<string>(registered)
  for (const provider of secretProviders) {
    const value = provider()?.trim()
    if (value && value.length >= MIN_SECRET_CHARS) values.add(value)
  }
  // Uzun değer önce: biri diğerini içeriyorsa parça kalmasın.
  return [...values].sort((a, b) => b.length - a.length)
}

/** Araç çıktısındaki API anahtarı/token benzeri değerleri modele ve arayüze gitmeden önce maskeler (CWE-200). */
export function maskSecrets(text: string): string {
  if (UNMASK) return text
  let out = text
  for (const secret of knownSecrets()) out = out.replaceAll(secret, `${MASK} (VENICE_API_KEY)`)
  for (const [pattern, replacement] of PATTERNS) out = out.replace(pattern, replacement)
  return out
}

/**
 * Maskeleme varsayılan AÇIK (uzun anahtarlar bağlamı şişirmesin, ekranda okunabilsin).
 * Ham değerleri görmek istersen: KOR_UNMASK=1 (değerler: 1, true, on, yes).
 */
export const UNMASK: boolean = /^(1|true|on|yes)$/i.test(process.env.KOR_UNMASK ?? process.env.VENICE_UNMASK ?? '')

/**
 * Ajanın ya da projenin kodunu çalıştıran alt süreçlere verilecek ortam: adı anahtar/token/parola/gizli içeren
 * değişkenler çıkarılır (VENICE_API_KEY, OPENAI_API_KEY, HF_TOKEN, ARTIFICIAL_ANALYSIS_API_KEY, GITHUB_TOKEN...).
 * Aksi halde modelin çalıştırdığı tek bir `env` komutu ya da bir istem enjeksiyonu uygulamanın anahtarlarını okurdu.
 */
export function scrubEnv(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(env).filter(([key]) => !/API_KEY|SECRET|TOKEN|PASSWORD|PASSWD/i.test(key)))
}
