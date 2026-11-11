// [g30] Otomatik hafıza: her kullanıcı turundan sonra arka planda ucuz bir modelle, o turdan kalıcı ve KULLANICIYA ÖZEL
// gerçekler (kararlı tercihler, ortam/donanım, projeler, uzun vadeli kararlar) çıkarılır. Çıkarım DETERMİNİSTİK çalışır:
// ana modelin "remember çağırayım mı" kararına bırakılmaz. Gizlilik: hassas konular ve sırlar dışlanır, hiçbir şey
// otomatik yazılmaz — her aday ONAY KUYRUĞUNA girer (kaydet/atla/düzelt). Onaylanınca global hafızaya (memory.md) eklenir.
import { randomBytes } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import type { Session } from './agent.ts'
import { fold } from './intel-g11.ts'
import { appendMemory, loadMemory } from './memory.ts'
import { scanSecrets } from './secret-scan.ts'
import { splitAttachments } from './sessions.ts'
import { titleModelFor } from './session-title.ts'
import { completeStructured } from './structured-output.ts'
import { USER_CONFIG_DIR } from './settings.ts'

export const PENDING_FILE = path.join(USER_CONFIG_DIR, 'memory-pending.json')
const CONFIG_FILE = path.join(USER_CONFIG_DIR, 'g30.json')
const EXTRACT_TIMEOUT_MS = 20_000
const MAX_PER_TURN = 3
const MAX_FACT_CHARS = 200
const MAX_PENDING = 50
const MAX_CONTEXT_CHARS = 4000

export interface PendingMemory {
  id: string
  text: string
  createdAt: number
}

const pendingSchema = z.object({
  version: z.literal(1),
  items: z.array(z.object({ id: z.string(), text: z.string(), createdAt: z.number() })).max(MAX_PENDING),
})

// Hassas konular: bu sözcükleri içeren adaylar hafızaya girmez (gizlilik). Kullanıcı yine elle ekleyebilir.
const SENSITIVE = /\b(sağlık|hasta|hastalık|teşhis|ilaç|din|dini|inanç|namaz|siyaset|siyasi|parti|oy verdi|cinsel|cinsellik|maaş|gelir|borç|kredi kartı|iban|tckn|kimlik no|hamile|depres|terapi)\b/i

const lastText = (session: Session, role: 'user' | 'assistant'): string => {
  for (let i = session.messages.length - 1; i >= 0; i--) {
    const m = session.messages[i]
    if (m?.role === role && typeof m.content === 'string' && m.content.trim()) {
      const text = role === 'user' ? splitAttachments(m.content).text : m.content
      if (text.trim()) return text.slice(0, MAX_CONTEXT_CHARS)
    }
  }
  return ''
}

// Şemaya zorlanan çıktı (structured-output.ts): bozuk JSON onarılır, olmazsa hata modele geri verilip yeniden denenir.
export const factsSchema = z.object({ facts: z.array(z.string().max(1000)).max(10) })

/** Adayları eler: boş/uzun, hassas, sır içeren, mevcut hafızada/önceki adaylarda olanlar atılır; en fazla MAX_PER_TURN. */
export function filterFacts(facts: readonly string[], existingMemory: string, existingPending: readonly string[]): string[] {
  const seen = new Set<string>()
  const haveFolded = fold(`${existingMemory} ${existingPending.join(' ')}`)
  const out: string[] = []
  for (const fact of facts) {
    if (!fact || fact.length < 4 || fact.length > MAX_FACT_CHARS) continue
    if (SENSITIVE.test(fact)) continue
    if (scanSecrets(fact).length > 0) continue
    const key = fold(fact)
    if (!key || seen.has(key)) continue
    if (haveFolded.includes(key)) continue // zaten biliniyor
    seen.add(key)
    out.push(fact)
    if (out.length >= MAX_PER_TURN) break
  }
  return out
}

/** O turdan kalıcı kullanıcı gerçekleri çıkarır (araçsız tek model çağrısı). Üretemezse/çıkaramazsa boş döner. */
export async function extractMemoryCandidates(session: Session, existingMemory: string, signal?: AbortSignal): Promise<string[]> {
  const user = lastText(session, 'user')
  const assistant = lastText(session, 'assistant')
  if (!user || !assistant) return []
  const model = titleModelFor(session)
  const result = await completeStructured({
    client: session.client,
    model,
    name: 'memory_facts',
    instructions:
      'Extract 0 to 3 durable, user-specific facts worth remembering across all future chats: stable preferences, ' +
      'environment/hardware, long-term projects, lasting decisions, how the user likes to work. ' +
      'Write each fact as a short standalone sentence in the user\'s language; return an empty "facts" array if nothing durable. ' +
      'NEVER include secrets, API keys, passwords or tokens. EXCLUDE sensitive topics (health, religion, politics, finances, sexuality). ' +
      'Exclude one-off task details, code, and anything transient.',
    input: `Current memory (do not repeat these):\n${existingMemory || '(empty)'}\n\nLatest exchange:\nUser: ${user}\n\nAssistant: ${assistant}`,
    schema: factsSchema,
    maxAttempts: 2,
    maxTokens: 300,
    temperature: 0.1,
    signal: AbortSignal.any([AbortSignal.timeout(EXTRACT_TIMEOUT_MS), session.lifetime.signal, ...(signal ? [signal] : [])]),
  })
  const prev = session.usage.get(model) ?? { promptTokens: 0, completionTokens: 0 }
  session.usage.set(model, {
    promptTokens: prev.promptTokens + result.usage.promptTokens,
    completionTokens: prev.completionTokens + result.usage.completionTokens,
  })
  return result.value.facts.map((s) => s.replace(/\s+/g, ' ').trim())
}

export async function loadPending(file: string = PENDING_FILE): Promise<PendingMemory[]> {
  try {
    const parsed = pendingSchema.safeParse(JSON.parse(await readFile(file, 'utf8')))
    return parsed.success ? parsed.data.items : []
  } catch {
    return []
  }
}

async function savePending(items: PendingMemory[], file: string): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, JSON.stringify({ version: 1, items: items.slice(-MAX_PENDING) }), { encoding: 'utf8', mode: 0o600 })
}

/** Yeni adayları onay kuyruğuna ekler (tekilleştirilmiş). Eklenen sayıyı döner. */
export async function addPending(texts: readonly string[], file: string = PENDING_FILE): Promise<number> {
  if (texts.length === 0) return 0
  const items = await loadPending(file)
  const haveFolded = new Set(items.map((i) => fold(i.text)))
  let added = 0
  for (const text of texts) {
    const key = fold(text)
    if (!key || haveFolded.has(key)) continue
    haveFolded.add(key)
    items.push({ id: `p${randomBytes(5).toString('hex')}`, text, createdAt: Date.now() })
    added++
  }
  if (added > 0) await savePending(items, file)
  return added
}

/** Bir adayı çözer: 'save' → (düzenlenmişse editedText ile) global hafızaya ekle; 'skip' → yalnızca kuyruktan çıkar. */
export async function resolvePending(
  id: string,
  action: 'save' | 'skip',
  editedText: string | undefined,
  pendingFile: string = PENDING_FILE,
  memoryFile?: string,
): Promise<void> {
  const items = await loadPending(pendingFile)
  const item = items.find((i) => i.id === id)
  if (!item) return
  if (action === 'save') {
    const text = (editedText ?? item.text).replace(/\s+/g, ' ').trim()
    if (text) await appendMemory(text, memoryFile)
  }
  await savePending(items.filter((i) => i.id !== id), pendingFile)
}

/** Otomatik hafıza açık mı (varsayılan açık). */
export async function isAutoMemoryEnabled(): Promise<boolean> {
  try {
    const cfg: unknown = JSON.parse(await readFile(CONFIG_FILE, 'utf8'))
    return (cfg as { autoMemory?: boolean })?.autoMemory !== false
  } catch {
    return true
  }
}

export async function setAutoMemoryEnabled(on: boolean): Promise<void> {
  await mkdir(USER_CONFIG_DIR, { recursive: true })
  await writeFile(CONFIG_FILE, JSON.stringify({ autoMemory: on }), { encoding: 'utf8', mode: 0o600 })
}

/** Tur sonu kancası: çıkar + ele + onay kuyruğuna ekle. Hata fırlatmaz (arka plan; sohbeti bozmaz). Eklenen sayıyı döner. */
export async function runAutoMemory(session: Session, signal?: AbortSignal): Promise<number> {
  try {
    if (!(await isAutoMemoryEnabled())) return 0
    const existing = (await loadMemory()) ?? ''
    const facts = await extractMemoryCandidates(session, existing, signal)
    const pending = (await loadPending()).map((p) => p.text)
    const kept = filterFacts(facts, existing, pending)
    return kept.length > 0 ? addPending(kept) : 0
  } catch {
    return 0
  }
}
