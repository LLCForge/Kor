// Sağlayıcı logosu kararının saf kısmı (§23.2, ADR 0053; test: provider-logo.test.ts).

export type LogoPlan =
  | { kind: 'local'; title: string; provider: string; modelId: string; via: null; monogram: string }
  | { kind: 'brand' | 'monogram'; title: string; provider: string; modelId: string; via: 'openrouter' | null; monogram: string }

/** Yerel çalışma zamanları (backend/providers/local*.toml, ollama). */
export function isLocalProvider(provider: string): boolean {
  return provider === 'local' || provider === 'ollama' || provider.startsWith('local-')
}

export function monogramOf(text: string): string {
  return (text.match(/[\p{L}\p{N}]/u)?.[0] ?? '?').toLocaleUpperCase('tr')
}

/**
 * OpenRouter model kimlikleri `<asıl-sağlayıcı>/<model>` biçimindedir: logo asıl sağlayıcınınki olur, OpenRouter küçük
 * rozet olarak kalır. `hasBrand` logonun bulunup bulunmadığını söyler (monogram düşüşü).
 */
export function logoPlan(provider: string, modelId: string, hasBrand: (provider: string, modelId: string) => boolean): LogoPlan {
  if (isLocalProvider(provider)) {
    return { kind: 'local', title: `${provider} (yerel)`, provider, modelId, via: null, monogram: monogramOf(provider) }
  }
  let real = provider
  let model = modelId
  let via: 'openrouter' | null = null
  if (provider === 'openrouter') {
    const slash = modelId.replace(/^openrouter\//, '').split('/')
    if (slash.length > 1 && slash[0]) {
      real = slash[0]
      model = slash.slice(1).join('/')
      via = 'openrouter'
    }
  }
  const title = via ? `${real} (OpenRouter üzerinden)` : real
  return { kind: hasBrand(real, model) ? 'brand' : 'monogram', title, provider: real, modelId: model, via, monogram: monogramOf(real) }
}
