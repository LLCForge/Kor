// Mesaj başlığındaki "<logo> deepseek-v4-1-flash yanıtladı" için (§23.2, ADR 0053): bir yanıtı hangi model verdi?
// Oturumun güncel modeli ve sohbetteki model değişikliği notlarından (g3:model → modelNote) geriye doğru çıkarılır:
// i. öğeden sonraki ilk değişikliğin `from`'u o yanıtın modelidir; sonra değişiklik yoksa güncel model.

export interface ModelNoteLike {
  kind: string
  from?: string
}

export function modelAt(items: readonly ModelNoteLike[], index: number, current: string): string {
  for (let i = index + 1; i < items.length; i++) {
    const it = items[i]!
    if (it.kind === 'modelNote' && typeof it.from === 'string' && it.from) return it.from
  }
  return current
}

/** Model kimliğinden sağlayıcı önekini atar (`openai/gpt-5` → `gpt-5`); öneki yoksa olduğu gibi. */
export function modelName(ref: string, provider: string): string {
  return ref.startsWith(`${provider}/`) ? ref.slice(provider.length + 1) : ref
}

/** Her yanıt öğesinin modeli (sondan başa tek geçiş): öğe kimliği → model kimliği. */
export function modelsByItem(items: readonly (ModelNoteLike & { id?: string })[], current: string): Map<string, string> {
  const out = new Map<string, string>()
  let model = current
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i]!
    if (it.kind === 'modelNote' && typeof it.from === 'string' && it.from) model = it.from
    else if (it.kind === 'assistant' && it.id) out.set(it.id, model)
  }
  return out
}
