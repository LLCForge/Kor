// Sağlayıcı/model marka ikonları. SVG'ler @lobehub/icons-static-svg'den (venice.ai
// model ikonlarıyla aynı set) + Venice'in kendi ikonu; hepsi currentColor + 1em, tema
// uyumlu. Markalar ilgili sahiplerinin ticari markalarıdır; yalnızca ilgili
// sağlayıcıyı/modeli tanımlamak için (nominative use) gösterilir.
import type { ReactElement } from 'react'
import type { CatalogModel } from '../../../core/model-catalog.ts'
import { modelProvider } from '../../../core/model-registry-g9.ts'
import type { ProviderId } from '../../../core/providers.ts'

import { PROVIDER_LOGOS } from '../provider-logos.gen.ts'
import { monogramOf } from '../provider-logo.ts'

// Logolar assets/providers/ altında (§23.2, ADR 0053; lisans notu assets/providers/LICENSES.md).
const logo = (id: string): string => PROVIDER_LOGOS[id]?.mono ?? ''
const venice = logo('venice')
const anthropic = logo('anthropic')
const claude = logo('claude')
const openai = logo('openai')
const deepseek = logo('deepseek')
const gemini = logo('gemini')
const google = logo('google')
const qwen = logo('qwen')
const grok = logo('grok')
const meta = logo('meta')
const nvidia = logo('nvidia')
const flux = logo('flux')
const kimi = logo('kimi')
const chatglm = logo('chatglm')
const aionlabs = logo('aionlabs')
const kling = logo('kling')
const openrouter = logo('openrouter')
const groq = logo('groq')
const mistral = logo('mistral')
const ollama = logo('ollama')
const together = logo('together')
const xai = logo('xai')

/**
 * Sağlayıcı manifest kimliği (backend/providers/*.toml) → logo kimliği (assets/providers/<id>.svg).
 * Listede olmayan sağlayıcı ember çemberde harf monogramıyla çizilir (ProviderLogo).
 */
export const PROVIDER_LOGO_ID: Readonly<Record<string, string>> = {
  anthropic: 'anthropic', azure: 'azure', bedrock: 'bedrock', cerebras: 'cerebras', codestral: 'mistral', cohere: 'cohere',
  dashscope: 'alibabacloud', 'dashscope-cn': 'alibabacloud', deepseek: 'deepseek', fal: 'fal', fireworks: 'fireworks',
  'github-models': 'github', google: 'google', groq: 'groq', huggingface: 'huggingface', 'local-lmstudio': 'lmstudio',
  'local-vllm': 'vllm', 'microsoft-foundry': 'azureai', mistral: 'mistral', moonshot: 'moonshot', nvidia: 'nvidia',
  ollama: 'ollama', openai: 'openai', openrouter: 'openrouter', perplexity: 'perplexity', sagemaker: 'aws',
  sambanova: 'sambanova', together: 'together', venice: 'venice', vertex: 'vertexai', xai: 'xai', zai: 'zai',
  'zai-coding': 'zai', zhipu: 'zhipu', abliteration: 'abliteration', arliai: 'arliai', mancer: 'mancer',
  uncensoredllm: 'uncensoredllm', 'local-localai': 'localai', local: 'ollama',
}

/** Ham SVG metnini currentColor + sabit boyutla gömer (CSP img-src sorunu olmaz). */
export function BrandSvg({ svg, size = 14 }: { svg: string; size?: number }) {
  return (
    <span
      className="brand-ic"
      style={{ fontSize: size, display: 'inline-flex', flex: 'none', lineHeight: 0 }}
      aria-hidden="true"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

/** Venice (Kor sağlayıcısı) marka ikonu. */
export function VeniceIcon({ size = 14 }: { size?: number }) {
  return <BrandSvg svg={venice} size={size} />
}

// Sağlayıcı kimliği → marka SVG.
const BY_PROVIDER: Partial<Record<ProviderId, string>> = Object.fromEntries(
  Object.entries(PROVIDER_LOGO_ID).map(([provider, id]) => [provider, logo(id)]),
)

// Model adı anahtar kelimesi → marka SVG (öncelik sırasıyla).
const BY_NAME: ReadonlyArray<[RegExp, string]> = [
  [/claude/i, claude],
  [/\b(gpt|o[1-4]|codex|chatgpt)\b/i, openai],
  [/deepseek/i, deepseek],
  [/gemini/i, gemini],
  [/\b(gemma|palm|bard)\b/i, google],
  [/qwen/i, qwen],
  [/grok/i, grok],
  [/\b(llama|meta)\b/i, meta],
  [/\b(nemotron|nvidia)\b/i, nvidia],
  [/\b(flux|black.?forest)\b/i, flux],
  [/\b(kimi|moonshot)\b/i, kimi],
  [/\b(glm|chatglm|zhipu|z-ai)\b/i, chatglm],
  [/\b(aion|aionlabs)\b/i, aionlabs],
  [/kling/i, kling],
  [/mistral|mixtral|magistral/i, mistral],
  [/xai/i, xai],
]

function svgForModel(model: CatalogModel): string | null {
  const id = modelProvider(model)
  const name = `${model.name} ${model.id}`
  for (const [re, svg] of BY_NAME) if (re.test(name)) return svg
  return BY_PROVIDER[id] ?? null
}

/** Model/sağlayıcıya göre marka ikonu; eşleşme yoksa null (metin rozeti kalır). */
export function brandIconFor(model: CatalogModel): ReactElement | null {
  const svg = svgForModel(model)
  return svg ? <BrandSvg svg={svg} /> : null
}

/** Serbest sağlayıcı/model metni (ör. katalogdaki `provider` + `model_id`) için marka SVG; yoksa null. */
export function brandSvgForText(provider: string, modelId: string): string | null {
  const byProvider = (BY_PROVIDER as Record<string, string | undefined>)[provider]
  if (byProvider) return byProvider
  const text = `${provider} ${modelId}`
  for (const [re, svg] of BY_NAME) if (re.test(text)) return svg
  return null
}

// Renkli varyantı koyu zeminde kaybolan markalar açık karoda gösterilir.
const DARK_MARKS: ReadonlySet<string> = new Set(['uncensoredllm', 'aionlabs'])

/**
 * Ayarlar > Bağdaştırıcılar: markanın kendi renkleriyle, her sağlayıcıda aynı boyutta karo. Resmî SVG'si olmayan
 * sağlayıcı nötr harf rozetiyle gösterilir (logo tahmin edilmez ya da yeniden çizilmez).
 */
export function ProviderTile({ id, label }: { id: string; label: string }) {
  const logoId = PROVIDER_LOGO_ID[id]
  const entry = logoId === undefined ? undefined : PROVIDER_LOGOS[logoId]
  if (!entry) {
    return (
      <span className="g9-logo-tile g9-logo-fallback" aria-hidden="true" title="Resmî logo yok">
        {monogramOf(label.replace(/^(Local|Yerel)\s*\(?/i, ''))}
      </span>
    )
  }
  return (
    <span className={`g9-logo-tile${logoId !== undefined && DARK_MARKS.has(logoId) ? ' light' : ''}`} aria-hidden="true">
      <BrandSvg svg={entry.color} size={18} />
    </span>
  )
}
