// [perf] İstek öncesi araç çıktısı budama: eski turlardaki uzun araç sonuçları (dosya içeriği, build günlüğü) modele
// kırpılmış gönderilir. session.messages değişmez (kayıt, dışa aktarma ve arayüz tam metni görür); yalnızca istek
// gövdesi hafifler. Son KEEP_TURNS kullanıcı turu olduğu gibi kalır: model üzerinde çalıştığı çıktıyı kaybetmez.
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'

/** Budanmış kopya → asıl oturum dizisi. Kimlikle oturum bulan katmanlar (g5 koruma, g9 canlı sayaç) bununla çözer. */
const aliases = new WeakMap<object, ChatCompletionMessageParam[]>()
export function originalMessagesOf(list: readonly ChatCompletionMessageParam[]): readonly ChatCompletionMessageParam[] {
  return aliases.get(list) ?? list
}

/** Oturum dizisinden türetilmiş istek dizisini (ör. metin protokolü) asıl diziye bağlar. */
export function aliasMessages(copy: readonly ChatCompletionMessageParam[], original: ChatCompletionMessageParam[]): void {
  if (copy !== original) aliases.set(copy, original)
}

/** Bu araçların çıktıları hiç kırpılmaz: skill talimatları ve araç kataloğu turlar boyunca "çalışma yöntemi"dir. */
export const PRUNE_EXEMPT_TOOLS = new Set(['use_skill', 'list_skills', 'tool_search', 'run_team'])

export const PRUNE_KEEP_TURNS = 3
export const PRUNE_MIN_CHARS = 1_500
export const PRUNE_HEAD_CHARS = 400

// Uygulamanın kullanıcı rolüyle eklediği metinler. Bu modül yaprak kalsın diye sabitler yazılı (testi kaynaklarıyla eşleştirir).
const APP_NOTE_PREFIXES = ['<system-reminder', '<task-notification', '<subdirectory-hints', '<read-image', 'Ajan denetimi · ', '[Önceki konuşmanın özeti]']

function isRealUserMessage(m: ChatCompletionMessageParam | undefined): boolean {
  if (m?.role !== 'user') return false
  const text = typeof m.content === 'string' ? m.content : (m.content.find((p) => p.type === 'text')?.text ?? '')
  return !APP_NOTE_PREFIXES.some((prefix) => text.startsWith(prefix))
}

/** Kırpılacak sınır: son KEEP_TURNS gerçek kullanıcı mesajından öncekiler aday. */
function cutIndex(messages: readonly ChatCompletionMessageParam[], keepTurns: number): number {
  let seen = 0
  for (let i = messages.length - 1; i >= 0; i--) {
    if (isRealUserMessage(messages[i])) {
      seen++
      if (seen === keepTurns) return i
    }
  }
  return 0
}

export function pruneOldToolOutputs(
  messages: readonly ChatCompletionMessageParam[],
  opts: { keepTurns?: number; minChars?: number; headChars?: number } = {},
): ChatCompletionMessageParam[] {
  const keepTurns = opts.keepTurns ?? PRUNE_KEEP_TURNS
  const minChars = opts.minChars ?? PRUNE_MIN_CHARS
  const head = opts.headChars ?? PRUNE_HEAD_CHARS
  const cut = cutIndex(messages, keepTurns)
  if (cut === 0) return messages as ChatCompletionMessageParam[]
  // tool_call_id → araç adı (muaf araçlar için)
  const names = new Map<string, string>()
  for (const m of messages) {
    if (m.role === 'assistant' && m.tool_calls) for (const c of m.tool_calls) if (c.type === 'function') names.set(c.id, c.function.name)
  }
  let changed = false
  const out = messages.map((m, i) => {
    if (i >= cut || m.role !== 'tool' || typeof m.content !== 'string' || m.content.length <= minChars) return m
    if (PRUNE_EXEMPT_TOOLS.has(names.get(m.tool_call_id) ?? '')) return m
    if (m.content.includes('[eski araç çıktısı kırpıldı')) return m
    changed = true
    const dropped = m.content.length - head
    return { ...m, content: `${m.content.slice(0, head)}\n…[eski araç çıktısı kırpıldı: ${dropped} karakter; gerekirse aracı yeniden çağır]` }
  })
  if (!changed) return messages as ChatCompletionMessageParam[]
  aliases.set(out, messages as ChatCompletionMessageParam[])
  return out
}
