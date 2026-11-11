// Bağlam sınırı için yedek tablo. Sağlayıcı model listeleri çoğu zaman bağlam boyu vermez (Anthropic, OpenAI, Groq),
// bu yüzden sınır bilinmeyince otomatik sıkıştırma hiç tetiklenmiyor ve bağlam sessizce taşıyordu.
// Sıra: sağlayıcı kataloğu → buradaki aile eşleşmesi → sağlayıcı varsayılanı → genel varsayılan.
// Değerler bilerek muhafazakâr: erken sıkıştırma, taşmadan iyidir.
import type { ProviderId } from './providers.ts'

/** Bilinen aileler (model kimliğinin sağlayıcı ön eki atılmış hâline uygulanır). */
const FAMILY_LIMITS: ReadonlyArray<readonly [RegExp, number]> = [
  // Anthropic: tüm güncel Claude modelleri 200k (1M beta başlıkla açılır, varsayılan değil).
  [/^claude/i, 200_000],
  // OpenAI: uzun bağlamlı aileler açıkça, kalanlar 128k.
  [/^gpt-4\.1/i, 1_000_000],
  [/^gpt-5|^o[34]/i, 200_000],
  [/^(gpt-4o|gpt-4|gpt-3\.5)/i, 128_000],
  // Google (OpenRouter/OpenAI uyumlu yollardan gelen) uzun bağlamlı modeller.
  [/^gemini-1\.5|^gemini-2/i, 1_000_000],
  [/^gemini/i, 128_000],
  // Yerel/açık modeller: çoğu 8k-32k pencerede çalışır.
  // Venice kataloğundaki gerçek değerler; `deepseek-ai/...` gibi başka sağlayıcı kimlikleri bunlara uymaz.
  [/^deepseek-v4/i, 1_000_000],
  [/^deepseek-v3[.-]2/i, 160_000],
  [/^(llama|mistral|mixtral|qwen|phi|gemma|deepseek)/i, 32_000],
]

/** Sağlayıcı varsayılanı: aile eşleşmesi yoksa. Küçük tutulur; erken sıkıştırma taşmadan iyidir. */
const PROVIDER_DEFAULT: Partial<Record<ProviderId, number>> = {
  anthropic: 200_000,
  openai: 128_000,
  groq: 128_000,
  openrouter: 128_000,
  deepinfra: 128_000,
  together: 128_000,
  fireworks: 128_000,
  huggingface: 128_000,
  venice: 32_000,
  veniceweb: 32_000,
}

/** Hiçbir bilgi yoksa kullanılan taban: sıkıştırma yine de çalışsın. */
const GLOBAL_DEFAULT = 32_000

/**
 * Model için yedek bağlam sınırı. `rawModel` sağlayıcı öneki atılmış ham kimlik olmalı.
 * Katalogda gerçek değer varsa çağıran onu tercih eder (context.ts).
 */
export function fallbackContextLimit(provider: ProviderId, rawModel: string): number {
  for (const [pattern, tokens] of FAMILY_LIMITS) if (pattern.test(rawModel)) return tokens
  return PROVIDER_DEFAULT[provider] ?? GLOBAL_DEFAULT
}
