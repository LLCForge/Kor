// Model kimliğinin solunda sağlayıcı logosu (§23.2): maliyet panosu, kota paneli, karar listeleri.
import { providerOf } from '../../core/providers.ts'
import { modelName } from './message-model.ts'
import { ProviderLogo } from './panels/providers/ProviderLogo.tsx'

/** `openai/gpt-5` → [OpenAI logosu] gpt-5 */
export function ModelRef({ model, className = 'g7-mono' }: { model: string; className?: string }) {
  const provider = providerOf(model)
  return (
    <span className="kor-model-ref" translate="no" title={model}>
      <ProviderLogo provider={provider} modelId={modelName(model, provider)} size={14} />
      <span className={className}>{model}</span>
    </span>
  )
}
