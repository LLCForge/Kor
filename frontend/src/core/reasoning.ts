// [g3] Akış yardımcıları: akıl yürütme parçaları ve model değişikliği notu.
import type { ContextState } from './context.ts'

/**
 * Akıl yürütme parçası. Venice/DeepSeek `delta.reasoning_content` (gerçek akışla doğrulandı), OpenRouter/Groq vb.
 * `delta.reasoning` alanını kullanır; ikisi de tek iç biçime (düz metin) indirgenir.
 */
export function reasoningDelta(chunk: unknown): string | null {
  if (typeof chunk !== 'object' || chunk === null || !('choices' in chunk) || !Array.isArray(chunk.choices)) return null
  const choice: unknown = chunk.choices[0]
  if (typeof choice !== 'object' || choice === null || !('delta' in choice)) return null
  const delta: unknown = choice.delta
  if (typeof delta !== 'object' || delta === null) return null
  const text = 'reasoning_content' in delta ? delta.reasoning_content : 'reasoning' in delta ? delta.reasoning : null
  return typeof text === 'string' && text.length > 0 ? text : null
}

const THINK_OPEN = '<think>'
const THINK_CLOSE = '</think>'

/** Metnin sonundaki, `tag`in başlangıcı olabilecek en uzun parça (parçalar arası bölünmüş etiket için bekletilir). */
function partialTagSuffix(text: string, tag: string): number {
  for (let n = Math.min(tag.length - 1, text.length); n > 0; n--) if (tag.startsWith(text.slice(-n))) return n
  return 0
}

/**
 * İçerik akışındaki satır içi `<think>…</think>` bloğunu (DeepSeek-R1/Qwen tarzı, Ollama vb.) ayırır: düşünce
 * onThink'e, kalan metin onText'e gider. Etiket parçalar arasında bölünse de doğru çalışır.
 * shortcut: yalnızca yanıtın BAŞINDAKİ (boşluk hariç) blok tanınır — sohbet içinde geçen "<think>" metni yanlışlıkla
 * gizlenmesin; açılış etiketi istemde kalan (yalnız `</think>` üreten) şablonlar desteklenmez.
 */
export function createThinkFilter(onText: (text: string) => void, onThink: (text: string) => void): { push(chunk: string): void; flush(): void } {
  let state: 'start' | 'think' | 'text' = 'start'
  let buf = ''
  const emitText = (t: string) => {
    if (t) onText(t)
  }
  const emitThink = (t: string) => {
    if (t) onThink(t)
  }
  return {
    push(chunk) {
      if (state === 'text') return emitText(chunk)
      buf += chunk
      if (state === 'start') {
        const trimmed = buf.trimStart()
        if (trimmed.startsWith(THINK_OPEN)) {
          state = 'think'
          buf = trimmed.slice(THINK_OPEN.length)
        } else if (THINK_OPEN.startsWith(trimmed)) {
          return // "<thi" gibi yarım etiket ya da yalnız boşluk: karar için sonraki parçayı bekle
        } else {
          state = 'text'
          const out = buf
          buf = ''
          return emitText(out)
        }
      }
      const end = buf.indexOf(THINK_CLOSE)
      if (end >= 0) {
        emitThink(buf.slice(0, end))
        state = 'text'
        const rest = buf.slice(end + THINK_CLOSE.length).replace(/^\s+/, '')
        buf = ''
        return emitText(rest)
      }
      const hold = partialTagSuffix(buf, THINK_CLOSE)
      emitThink(buf.slice(0, buf.length - hold))
      buf = buf.slice(buf.length - hold)
    },
    flush() {
      if (state === 'think') emitThink(buf)
      else emitText(buf)
      buf = ''
      state = 'text'
    },
  }
}

/** Kalıcı geçmiş için: yanıtın başındaki düşünce bloğu atılır (geri gönderilip bağlamı şişirmesin). */
export function stripThink(text: string): string {
  const trimmed = text.trimStart()
  if (!trimmed.startsWith(THINK_OPEN)) return text
  const end = trimmed.indexOf(THINK_CLOSE)
  return end < 0 ? '' : trimmed.slice(end + THINK_CLOSE.length).replace(/^\s+/, '')
}

/** Model oturum ortasında değiştiyse (arayüzden seçildiyse) bir sonraki çağrıdan önce bildirilir. */
export function noteModelChange(
  session: { model: string; g3LastModel?: string | undefined; contextState?: ContextState | undefined },
  onChange: ((from: string, to: string) => void) | undefined,
): void {
  const changed = session.g3LastModel !== undefined && session.g3LastModel !== session.model
  if (changed) {
    // Bağlam durumu eski modelin sınırına göre kurulmuştu: yeni modele göre sıfırlanır.
    if (session.contextState) {
      session.contextState.lastPromptTokens = 0
      session.contextState.lastMessageCount = 0
      session.contextState.tailTokens = null
      session.contextState.tailCount = 0
      session.contextState.minNextCompact = 0
      session.contextState.activeReduction = 0
    }
    onChange?.(session.g3LastModel ?? '', session.model)
  }
  session.g3LastModel = session.model
}

// [g2] Akıl yürütme düzeyi seçici (aynı dosya adını iki grup açmıştı; birleştirmede tek modülde toplandı).

/**
 * Venice chat/completions `reasoning_effort` değerlerinin arayüzde sunulan alt kümesi (docs.venice.ai).
 * Modelin kabul ettiği değerler /models yanıtındaki capabilities.reasoningEffortOptions alanındadır.
 */
// 'none' düşünmeyi kapatır (sıfır düşünme bütçesi), 'max' en geniş bütçe; seçici yalnızca modelin desteklediklerini gösterir.
export const REASONING_EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as const
export type ReasoningEffortLevel = (typeof REASONING_EFFORTS)[number]

export const REASONING_EFFORT_LABELS: Readonly<Record<ReasoningEffortLevel, string>> = {
  none: 'Kapalı',
  minimal: 'Minimal',
  low: 'Düşük',
  medium: 'Orta',
  high: 'Yüksek',
  xhigh: 'Çok yüksek',
  max: 'Maksimum',
}

export const isReasoningEffort = (value: string): value is ReasoningEffortLevel =>
  (REASONING_EFFORTS as readonly string[]).includes(value)
