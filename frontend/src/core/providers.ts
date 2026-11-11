/**
 * LLM sağlayıcı kayıt defteri. Hepsi OpenAI uyumlu uç noktalar sunduğu için tek `openai` SDK'siyle,
 * yalnızca baseURL değiştirilerek kullanılır (Anthropic da OpenAI uyumlu uç noktasıyla).
 * Model kimliği "<provider>:<model>" biçiminde ad alanlıdır; ön eki olmayan kimlik geriye uyumlu olarak Venice'tir.
 */
export const PROVIDER_IDS = [
  'venice',
  'veniceweb',
  'openai',
  'anthropic',
  'openrouter',
  'groq',
  'experiential',
  'uplinkswitch',
  'abliteration',
  'featherless',
  'arliai',
  'infermatic',
  'deepinfra',
  'fireworks',
  'together',
  'huggingface',
  'ollama',
] as const
export type ProviderId = (typeof PROVIDER_IDS)[number]

export interface Provider {
  id: ProviderId
  label: string
  baseURL: string
  /** CLI/ortam değişkeni adı. */
  keyEnv: string
  /** Modeller uç noktası (baseURL'e görece). Venice tür filtresi ister. */
  modelsPath: string
  /** Yalnızca Venice bu alanları/başlıkları destekler (venice_parameters, prompt_cache_key, hız limiti başlıkları). */
  venice: boolean
  /** Anahtar gerektirmez (yerel Ollama); kullanıcı açıkça etkinleştirmeden kullanılmaz. */
  keyless: boolean
  /** Sansürsüz model sunar (ayarlarda rozet olarak gösterilir). */
  uncensored: boolean
  /**
   * Araç (function) çağırmayı desteklemez: girdi `tools` alanı yok sayılır, `role:"tool"` mesajları
   * düz metne katlanır. Sağlayıcı katmanı bu sağlayıcılara ait modellere tools göndermez.
   */
  noTools?: boolean
  /** OpenAI uyumlu olmayan, kendi taşımasıyla çalışan yerel köprü (bkz. venice-web.ts). */
  bridge?: boolean
}

export const PROVIDERS: readonly Provider[] = [
  { id: 'venice', label: 'Kor', baseURL: 'https://api.venice.ai/api/v1', keyEnv: 'VENICE_API_KEY', modelsPath: '/models?type=text', venice: true, keyless: false, uncensored: true },
  // Venice web hesabı: API anahtarı yerine e-posta+parola+2FA ile giriş. İşlem içi köprü, port açmaz.
  { id: 'veniceweb', label: 'Kor Web Hesabı', baseURL: 'http://127.0.0.1:8766/v1', keyEnv: 'VENICE_WEB_EMAIL', modelsPath: '/models', venice: false, keyless: false, uncensored: true, noTools: true, bridge: true },
  { id: 'openai', label: 'OpenAI', baseURL: 'https://api.openai.com/v1', keyEnv: 'OPENAI_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  { id: 'anthropic', label: 'Anthropic', baseURL: 'https://api.anthropic.com/v1', keyEnv: 'ANTHROPIC_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  { id: 'openrouter', label: 'OpenRouter', baseURL: 'https://openrouter.ai/api/v1', keyEnv: 'OPENROUTER_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: true },
  { id: 'groq', label: 'Groq', baseURL: 'https://api.groq.com/openai/v1', keyEnv: 'GROQ_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  // Adresler sağlayıcıların resmi belgelerinden (2026-10). "uncensored": belgelerde sansürsüz/abliterated model var.
  { id: 'experiential', label: 'Experiential Labs', baseURL: 'https://api.experientiallabs.ai/v1', keyEnv: 'EXPLABS_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: true },
  // Sitesi uplinkswitch.ai, API alan adı .com.
  { id: 'uplinkswitch', label: 'UplinkSwitch', baseURL: 'https://api.uplinkswitch.com/v1', keyEnv: 'UPLINKSWITCH_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: true },
  { id: 'abliteration', label: 'abliteration.ai', baseURL: 'https://api.abliteration.ai/v1', keyEnv: 'ABLITERATION_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: true },
  // Tam liste 22 binden fazla model (~7 MB): planda kullanılabilir olanlar, en fazla 1000.
  { id: 'featherless', label: 'Featherless', baseURL: 'https://api.featherless.ai/v1', keyEnv: 'FEATHERLESS_API_KEY', modelsPath: '/models?available_on_current_plan=true&per_page=1000', venice: false, keyless: false, uncensored: true },
  { id: 'arliai', label: 'ArliAI', baseURL: 'https://api.arliai.com/v1', keyEnv: 'ARLIAI_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: true },
  // Infermatic'in API alan adı totalgpt.ai.
  { id: 'infermatic', label: 'Infermatic', baseURL: 'https://api.totalgpt.ai/v1', keyEnv: 'INFERMATIC_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  { id: 'deepinfra', label: 'DeepInfra', baseURL: 'https://api.deepinfra.com/v1/openai', keyEnv: 'DEEPINFRA_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  { id: 'fireworks', label: 'Fireworks', baseURL: 'https://api.fireworks.ai/inference/v1', keyEnv: 'FIREWORKS_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  { id: 'together', label: 'Together AI', baseURL: 'https://api.together.ai/v1', keyEnv: 'TOGETHER_API_KEY', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  // Inference Providers yönlendiricisi; "Make calls to Inference Providers" izinli ince ayarlı token gerekir.
  { id: 'huggingface', label: 'Hugging Face', baseURL: 'https://router.huggingface.co/v1', keyEnv: 'HF_TOKEN', modelsPath: '/models', venice: false, keyless: false, uncensored: false },
  { id: 'ollama', label: 'Ollama (yerel)', baseURL: 'http://localhost:11434/v1', keyEnv: 'OLLAMA_API_KEY', modelsPath: '/models', venice: false, keyless: true, uncensored: false },
]

export const DEFAULT_PROVIDER: ProviderId = 'venice'

/** Venice web köprüsü anahtar gerektirmez; sağlayıcı havuzuna verilen yer tutucu anahtar. */
export const VENICEWEB_PLACEHOLDER_KEY = 'venice-web-session'

const byId = new Map<string, Provider>(PROVIDERS.map((p) => [p.id, p]))

/**
 * Köprü sağlayıcılarının taşıması burada tutulur; provider-pool köprü modülünü doğrudan import etmesin
 * (döngüsel bağımlılık: venice-web.ts → sağlayıcı havuzu yardımcıları).
 */
const bridgeFetches = new Map<string, (input: string | URL | Request, init?: RequestInit) => Promise<Response>>()

export function registerBridgeFetch(id: string, fn: (input: string | URL | Request, init?: RequestInit) => Promise<Response>): void {
  bridgeFetches.set(id, fn)
}

export function bridgeFetch(id: string): (input: string | URL | Request, init?: RequestInit) => Promise<Response> {
  const fn = bridgeFetches.get(id)
  if (!fn) return async () => new Response(JSON.stringify({ error: { message: `Köprü taşıması kayıtlı değil: ${id}` } }), { status: 500, headers: { 'content-type': 'application/json' } })
  return fn
}

export function getProvider(id: string): Provider | undefined {
  return byId.get(id)
}

/** Kayıt defterinde her kimlik vardır; bulunamazsa programlama hatasıdır. */
export function providerInfo(id: ProviderId): Provider {
  const p = byId.get(id)
  if (!p) throw new Error(`Bilinmeyen sağlayıcı: ${id}`)
  return p
}

/** "<provider>:<model>" ayrıştırır. Bilinen ön eki yoksa Venice kabul edilir (geriye uyumlu). Ollama etiketleri ("llama3:8b") ön ek sayılmaz. */
export function splitModelId(fullId: string): { provider: Provider; model: string } {
  const idx = fullId.indexOf(':')
  if (idx > 0) {
    const p = byId.get(fullId.slice(0, idx))
    if (p) return { provider: p, model: fullId.slice(idx + 1) }
  }
  return { provider: providerInfo('venice'), model: fullId }
}

/** Sağlayıcı ve ham model kimliğinden tam kimlik üretir; Venice ön eksiz kalır. */
export function makeModelId(providerId: ProviderId, model: string): string {
  return providerId === 'venice' ? model : `${providerId}:${model}`
}

export function providerOf(fullId: string): ProviderId {
  return splitModelId(fullId).provider.id
}

/** İstek gövdesine yazılacak ham model kimliği (sağlayıcı ön eki olmadan). */
export function apiModelId(fullId: string): string {
  return splitModelId(fullId).model
}

export function isVeniceModel(fullId: string): boolean {
  return splitModelId(fullId).provider.venice
}

/** Düz http yalnızca bu makineye izinli: anahtar veya istem yerel ağda açık metin dolaşmasın. */
export function isLoopbackHost(hostname: string): boolean {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '::1' || hostname === '[::1]'
}
