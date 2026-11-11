import type { Session } from './agent.ts'
import { splitAttachments } from './sessions.ts'

export const MAX_TITLE_WORDS = 6
const MAX_TITLE_CHARS = 60
const MAX_CONTEXT_CHARS = 1500
const TITLE_TIMEOUT_MS = 20_000

/** Başlık modeli tek yerde seçilir; ayarlardan (ör. taskModels) okunacaksa yalnızca burası değişir. */
export function titleModelFor(session: Pick<Session, 'model' | 'taskModels'>): string {
  return session.taskModels?.title ?? session.model
}

/** Model çıktısı güvenilmez: düşünce blokları, tırnaklar, "Başlık:" öneki ve fazla kelimeler atılır. */
export function cleanTitle(raw: string): string | null {
  const withoutThinking = raw.replace(/<think>[\s\S]*?(<\/think>|$)/gi, ' ')
  const line = withoutThinking
    .split('\n')
    .map((l) => l.trim())
    .find((l) => l.length > 0)
  if (!line) return null
  const words = line
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .replace(/^(?:başlık|title)\s*:\s*/i, '')
    .replace(/["'“”‘’«»`*_#]+/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[.!?:;,\-–—\s]+$/u, '')
    .split(' ')
    .filter(Boolean)
    .slice(0, MAX_TITLE_WORDS)
  const title = words.join(' ')
  if (!title) return null
  return title.length > MAX_TITLE_CHARS ? `${title.slice(0, MAX_TITLE_CHARS - 1).trimEnd()}…` : title
}

function firstText(session: Session, role: 'user' | 'assistant'): string {
  const m = session.messages.find((x) => x.role === role && typeof x.content === 'string' && x.content.trim())
  if (!m || typeof m.content !== 'string') return ''
  const text = role === 'user' ? splitAttachments(m.content).text : m.content
  return text.slice(0, MAX_CONTEXT_CHARS)
}

/**
 * İlk kullanıcı mesajı ve ilk asistan yanıtından en fazla 6 kelimelik Türkçe başlık üretir.
 * Kısa, araçsız tek çağrı; harcanan token oturum kullanımına eklenir. Üretilemezse null.
 */
export async function generateTitle(session: Session, signal?: AbortSignal): Promise<string | null> {
  const user = firstText(session, 'user')
  const assistant = firstText(session, 'assistant')
  if (!user || !assistant) return null
  const model = titleModelFor(session)
  const body = {
    model,
    messages: [
      {
        role: 'system' as const,
        content:
          'Bir sohbet için kısa bir başlık yaz. Kurallar: Türkçe, en fazla 6 kelime, tırnak işareti, emoji ve sonda noktalama yok. Yalnızca başlığı yaz, başka hiçbir şey yazma.',
      },
      { role: 'user' as const, content: `Kullanıcının ilk mesajı:\n${user}\n\nAsistanın ilk yanıtı:\n${assistant}` },
    ],
    max_completion_tokens: 24,
    temperature: 0.2,
    stream: false as const,
    // Düşünen modellerde 24 tokenın düşünceye harcanmaması için (Venice belgelerindeki alanlar).
    venice_parameters: { include_venice_system_prompt: false, disable_thinking: true, strip_thinking_response: true },
  }
  const timeout = AbortSignal.timeout(TITLE_TIMEOUT_MS)
  const completion = await session.client.chat.completions.create(body, {
    signal: AbortSignal.any([timeout, session.lifetime.signal, ...(signal ? [signal] : [])]),
    maxRetries: 1,
  })
  if (completion.usage) {
    const prev = session.usage.get(model) ?? { promptTokens: 0, completionTokens: 0 }
    session.usage.set(model, {
      promptTokens: prev.promptTokens + completion.usage.prompt_tokens,
      completionTokens: prev.completionTokens + completion.usage.completion_tokens,
    })
  }
  const content = completion.choices[0]?.message.content
  return typeof content === 'string' ? cleanTitle(content) : null
}
