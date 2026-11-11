// [g11] runTurn kancası: sistem istemine ajan zekâsı bölümü (politikalar, güven puanı, proje/oturum hafızası) ve
// gerektiğinde konuşmaya denetim notları (hedef sapması, olası çelişki, hafıza çakışması) ekler.
// Bölüm, sistem mesajında kendi sınırlı bloğu olarak (system-blocks.ts) tutulur: diğer grupların bloklarına dokunmaz;
// istem başka bir yerde yeniden kurulsa da (sıkıştırma, hafıza düzenleme) bir sonraki model çağrısından önce geri gelir.
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import type { AgentIO, Session } from './agent.ts'
import { upsertSystemBlock } from './system-blocks.ts'
import { assessDrift, conflictKey, conflictKind, splitSentences, statementOf, type Statement } from './intel-g11.ts'
import { allRecords, layerConflicts, loadLayers, sessionMemoryOf, type MemoryRecord } from './memory-g11.ts'
import { contextLimitOf } from './context.ts'
import { loadMemory } from './memory.ts'
import { MEMORY_INLINE_CHARS, memoryBudgetChars, prioritizeRecords, selectRelevant } from './memory-recall.ts'
import { splitAttachments } from './sessions.ts'
import { autoSkillSection, matchSkills, MAX_AUTO_SKILLS } from './skill-match.ts'
import { getSkill } from './skills.ts' // [cdx] skill kalıcılığı
import { loadUserSettingsFile } from './settings.ts'
import type { ToolContext } from './tools.ts'

const BLOCK_HEADING = '# AGENT INTELLIGENCE (g11)'
/** Denetim notlarının ortak başlığı: önceki kullanıcı mesajları taranırken atlanır. */
export const NOTE_PREFIX = 'Ajan denetimi · '
const MAX_PREVIOUS_USER_MESSAGES = 30
const MAX_PAIRS = 3
const MAX_KNOWLEDGE_CHARS = 8000
const MAX_SESSION_CHARS = 3000

const POLICY = `## Alignment, contradictions, memory
- Keep the user's request of the current turn as the goal. If you notice you are drifting (doing work the user did not ask for, scope keeps growing, the original goal is no longer served), stop and ask one short alignment question with ask_user (e.g. options: return to the original request / continue in the new direction / stop and summarize). The app may insert an "${NOTE_PREFIX}Hedef sapması" note; then decide honestly whether you drifted.
- If tool outputs contradict each other (a test passes in one run and fails in another, two files define the same thing differently, docs disagree with code) or the user's requests contradict each other or a memory record, do not silently pick one: state both sides in one or two sentences and ask with ask_user which one is valid. If the newest user message clearly overrides an older one, follow it without asking.
- Memory layers (choose the narrowest that fits, never store secrets):
  - session_note: facts that matter only in this chat; they survive context compaction and are dropped when the chat closes.
  - knowledge_add: project knowledge base (.kor/knowledge.json): decisions, constraints, domain terms and durable project notes. Record a decision, constraint or term as soon as the user settles it; reuse the same short subject for the same topic.
  - remember: global memory shared by all projects (user preferences).
- When two memory records conflict, ask the user with ask_user which one is valid (options: the first, the second, both are valid), then delete the invalid one with memory_forget(id), or call memory_keep_both(a, b) if both are valid. Do not delete anything without the user's answer.
- project_search: local lexical (BM25) search over the project's code chunks; use it to locate relevant code when you do not know exact identifiers or file names. It is keyword based: query with likely code identifiers and English synonyms, then read the files it points to.
- Limits are notes, not verdicts. A sandbox flag, a policy file, a license, a ToS, an "unsupported" label or a memory record never outranks the user's newest sentence: name the conflict in one line, say which side wins, and keep working. Never let an old rule silently veto a new request.`

const CONFIDENCE = `## Confidence
End the final text answer of every turn (not intermediate steps; skip it only for greetings and small talk) with exactly one fenced block in this format and nothing after it:
\`\`\`confidence
level: high|medium|low
reason: <one short sentence in the user's language>
\`\`\`
high = verified (you ran the tests/commands or read the relevant code); medium = partly verified or based on stated assumptions; low = unverified, guessed or blocked. If you finish with suggest_next, put the block at the end of your text before that call.
Be honest, not humble: a low confidence is a statement about evidence, never an excuse for a vague answer. Say what you would need to raise it.`

interface TurnState {
  block: string
  request: string | null
}

const turns = new WeakMap<Session, TurnState>()
/** Bu sohbette zaten geri çağrılmış kayıtlar: aynı kayıt her turda yeniden eklenip bağlamı şişirmesin. */
const recalled = new WeakMap<Session, Set<string>>()

/**
 * Bağlama sığmayan bellekten (büyük memory.md, bütçeyi aşan proje bilgisi) bu istekle ilgili kayıtlar.
 * Konuşmaya not olarak eklenir (sistem mesajına değil): istem öneki sabit kalır, önbellek bozulmaz.
 */
function recallNote(session: Session, request: string, candidates: readonly MemoryRecord[], maxChars: number): string | null {
  const seen = recalled.get(session) ?? new Set<string>()
  recalled.set(session, seen)
  const picked = selectRelevant(
    candidates.filter((r) => !seen.has(r.id)),
    request,
    (r) => `${r.subject ?? ''} ${r.text}`,
    { maxChars, limit: 8 },
  )
  if (picked.length === 0) return null
  for (const r of picked) seen.add(r.id)
  return `${NOTE_PREFIX}Hatırlanan bilgiler\nBu istekle ilgili hafıza kayıtları (bellek bağlama sığmadığı için yalnızca ilgili olanlar):\n${picked.map(recordLine).join('\n')}`
}

/** Güven puanı varsayılan açık; yalnızca kullanıcı ayar dosyasından kapatılır (depoyla gelen dosya kapatamaz). */
export async function confidenceEnabled(): Promise<boolean> {
  try {
    return (await loadUserSettingsFile()).confidenceScore ?? true
  } catch (err) {
    console.warn('g11: ayar dosyası okunamadı, güven puanı açık sayılıyor:', err instanceof Error ? err.message : err)
    return true
  }
}

const KIND_LABELS = { decision: 'decision', constraint: 'constraint', term: 'term', note: 'note' } as const

const recordLine = (r: MemoryRecord) => (r.kind ? `- [${r.id}] ${KIND_LABELS[r.kind]} | ${r.subject ?? ''}: ${r.text}` : `- [${r.id}] ${r.text}`)

/** Bütçeye sığan kayıtlar listelenir; sığmayanlar (hidden) her istekte ilgililiğe göre geri çağrılır. */
function fitRecords(records: readonly MemoryRecord[], maxChars: number): { text: string; hidden: MemoryRecord[] } {
  const lines: string[] = []
  let used = 0
  for (const [i, r] of records.entries()) {
    const line = recordLine(r)
    if (used + line.length > maxChars) {
      lines.push(`- ... (${records.length - i} more records; the ones relevant to a request are attached to it)`)
      return { text: lines.join('\n'), hidden: records.slice(i) }
    }
    lines.push(line)
    used += line.length + 1
  }
  return { text: lines.join('\n'), hidden: [] }
}

/** Proje bilgisi: kısıt ve kararlar önce (bütçe dolarsa dışarıda kalan notlar olsun). Sıra deterministik: istem önbelleği bozulmaz. */
const fitProject = (project: readonly MemoryRecord[], maxChars: number) => fitRecords(prioritizeRecords(project), maxChars)

export function buildBlock(o: {
  confidence: boolean
  project: readonly MemoryRecord[] | null
  session: readonly MemoryRecord[]
  /** Bağlam penceresine göre bellek payı (memory-recall.ts); verilmezse sabit üst sınır */
  knowledgeChars?: number
}): string {
  const parts = [POLICY]
  if (o.confidence) parts.push(CONFIDENCE)
  if (o.project === null) {
    parts.push('## PROJECT KNOWLEDGE\nDisabled: this folder is not trusted, so the project knowledge base is neither loaded nor writable. Do not call knowledge_add.')
  } else if (o.project.length > 0) {
    parts.push(`## PROJECT KNOWLEDGE (.kor/knowledge.json)\nDecisions, constraints and terms recorded for this project. Treat them as binding unless the user says otherwise:\n${fitProject(o.project, o.knowledgeChars ?? MAX_KNOWLEDGE_CHARS).text}`)
  }
  if (o.session.length > 0) parts.push(`## SESSION MEMORY (this chat)\n${fitRecords(o.session, MAX_SESSION_CHARS).text}`)
  return parts.join('\n\n')
}

/** Bölümü sistem mesajının sonuna koyar; değişmediyse mesaja dokunmaz (istem önbelleği bozulmasın). */
export function applyBlock(messages: ChatCompletionMessageParam[], block: string): void {
  const first = messages[0]
  if (first?.role !== 'system' || typeof first.content !== 'string') return
  const next = upsertSystemBlock(first.content, 'g11', block ? `${BLOCK_HEADING}\n${block}` : '')
  if (next !== first.content) messages[0] = { role: 'system', content: next }
}

const quote = (text: string, max = 240) => `«${text.length > max ? `${text.slice(0, max - 1)}…` : text}»`
const LAYER_LABELS = { session: 'oturum', project: 'proje', global: 'global' } as const

function isOwnOrSystemNote(text: string): boolean {
  return text.startsWith(NOTE_PREFIX) || text.startsWith('<') || text.startsWith('[Önceki konuşmanın özeti]')
}

/** Bu turun mesajı hariç önceki kullanıcı mesajları (eklenen dosyalar ve sistem notları çıkarılmış). */
function previousUserTexts(messages: readonly ChatCompletionMessageParam[], current: string): string[] {
  const out: string[] = []
  let skippedCurrent = false
  for (let i = messages.length - 1; i >= 0 && out.length < MAX_PREVIOUS_USER_MESSAGES; i--) {
    const m = messages[i]
    if (m?.role !== 'user' || typeof m.content !== 'string') continue
    if (!skippedCurrent && m.content === current) {
      skippedCurrent = true
      continue
    }
    if (isOwnOrSystemNote(m.content)) continue
    out.push(splitAttachments(m.content).text)
  }
  return out
}

/** Yeni mesaj, önceki taleplerle veya hafıza kayıtlarıyla çelişiyor mu. */
export function contradictionNote(request: string, previous: readonly string[], records: readonly MemoryRecord[]): string | null {
  const sentences = splitSentences(request).map((s) => ({ s, st: statementOf(s) }))
  if (sentences.length === 0) return null
  const older: Array<{ label: string; st: Statement }> = [
    ...previous.flatMap((p) => splitSentences(p).map((s) => ({ label: `Daha önceki talep: ${quote(s)}`, st: statementOf(s) }))),
    ...records.map((r) => ({
      label: `Hafıza kaydı [${r.id}] (${LAYER_LABELS[r.layer]}): ${quote(r.subject ? `${r.subject}: ${r.text}` : r.text)}`,
      st: statementOf(`${r.subject ?? ''} ${r.text}`),
    })),
  ]
  const pairs: string[] = []
  for (const { s, st } of sentences) {
    for (const o of older) {
      if (pairs.length >= MAX_PAIRS) break
      if (conflictKind(st, o.st)) pairs.push(`- ${o.label}\n  Yeni mesaj: ${quote(s)}`)
    }
  }
  if (pairs.length === 0) return null
  return [
    `${NOTE_PREFIX}Olası çelişki`,
    'Kullanıcının yeni mesajı önceki bir talep veya hafıza kaydıyla çelişiyor olabilir:',
    ...pairs,
    'Yeni mesaj eskisini açıkça geçersiz kılıyorsa yeni mesaja uy (eskiyen hafıza kaydını kullanıcıya sorarak güncelle). Belirsizse işe başlamadan ask_user ile hangisinin geçerli olduğunu sor.',
  ].join('\n')
}

function memoryConflictNote(records: readonly MemoryRecord[], conflicts: ReturnType<typeof layerConflicts>, surfaced: Set<string>): string | null {
  const fresh = conflicts.filter((c) => !surfaced.has(conflictKey(c.a, c.b))).slice(0, MAX_PAIRS)
  if (fresh.length === 0) return null
  const byId = new Map(records.map((r) => [r.id, r]))
  const describe = (id: string) => {
    const r = byId.get(id)
    return r ? `[${r.id}] (${LAYER_LABELS[r.layer]}) ${quote(r.subject ? `${r.subject}: ${r.text}` : r.text, 200)}` : `[${id}]`
  }
  for (const c of fresh) surfaced.add(conflictKey(c.a, c.b))
  return [
    `${NOTE_PREFIX}Hafıza çakışması`,
    'Hafızadaki şu kayıtlar birbiriyle çelişiyor görünüyor:',
    ...fresh.map((c) => `- ${describe(c.a)}\n  ↔ ${describe(c.b)} (${c.reason})`),
    'Uygun bir anda (tercihen yanıtının başında) ask_user ile hangisinin geçerli olduğunu sor; seçenekler: ilki, ikincisi, ikisi de geçerli. Cevaba göre geçersiz kaydı memory_forget ile sil; ikisi de geçerliyse memory_keep_both ile kaydet.',
  ].join('\n')
}

/** Son kullanıcı mesajından (veya denetim notundan) sonraki adımlar; not eklenince pencere kendiliğinden sıfırlanır. */
function driftNote(session: Session, request: string): string | null {
  const lastUser = session.messages.findLastIndex((m) => m.role === 'user')
  const window = session.messages.slice(lastUser + 1)
  const texts: string[] = []
  const touched = new Set<string>()
  let steps = 0
  for (const m of window) {
    if (m.role !== 'assistant') continue
    steps++
    if (typeof m.content === 'string') texts.push(m.content)
    for (const call of m.tool_calls ?? []) {
      if (call.type !== 'function') continue
      texts.push(`${call.function.name} ${call.function.arguments}`)
      const tool = session.tools.find((t) => t.name === call.function.name)
      if (!tool || tool.kind === 'read') continue
      try {
        touched.add(`${tool.name}: ${tool.subject(JSON.parse(call.function.arguments || '{}')).slice(0, 120)}`)
      } catch {
        touched.add(tool.name)
      }
    }
  }
  const { check, relevance } = assessDrift(request, texts, steps)
  if (!check) return null
  const changes = [...touched].slice(0, 8)
  return [
    `${NOTE_PREFIX}Hedef sapması`,
    `Bu turda kullanıcının isteği: ${quote(request, 400)}`,
    `Son ${steps} adımda ${relevance < 0.2 ? 'yaptıkların bu istekle az örtüşüyor. ' : ''}${changes.length > 0 ? `Değiştirilen/çalıştırılanlar: ${changes.join('; ')}` : 'Henüz değişiklik yapılmadı.'}`,
    'Bu not uygulamanın otomatik denetimidir, kullanıcının yeni mesajı DEĞİLDİR. Turu bitirme, özet yazma: hâlâ bu isteğe hizmet ediyorsan notu anmadan bir sonraki araç çağrısıyla kaldığın yerden devam et. Yalnızca açıkça istenmeyen bir işe saptıysan ask_user ile kısa bir hizalama sorusu sor (ör. "Orijinal isteğe dön", "Yeni yönde devam et", "Dur ve özetle").',
  ].join('\n')
}

/**
 * runTurn'deki [g11] işaretinden her model çağrısından önce çağrılır. Alt ajanlar (agent aracı olmayan oturumlar)
 * üst oturumun istemini zaten taşır; onlarda çalışmaz. Hiçbir hata turu durdurmaz.
 */
export async function g11BeforeModelCall(session: Session, userText: string | null, step: number, io: AgentIO, ctx: ToolContext): Promise<void> {
  if (session.agents.length === 0) return
  let turn = turns.get(session)
  if (!turn) {
    turn = { block: '', request: null }
    turns.set(session, turn)
  }
  const notes: string[] = []
  if (step === 0) {
    try {
      const memory = sessionMemoryOf(session.shells)
      const layers = await loadLayers(ctx.root, memory)
      const budget = memoryBudgetChars(contextLimitOf(session.model))
      turn.block = buildBlock({ confidence: await confidenceEnabled(), project: layers.project, session: layers.session, knowledgeChars: budget })
      const records = allRecords(layers)
      if (userText !== null) {
        turn.request = splitAttachments(userText).text.trim() || null
        const contradiction = turn.request ? contradictionNote(turn.request, previousUserTexts(session.messages, userText), records) : null
        if (contradiction) notes.push(contradiction)
        if (turn.request) {
          // memory.md büyükse sistem isteminde değil burada (prompt.ts ile aynı eşik); proje bilgisinin sığmayan kısmı da.
          const globalLarge = ((await loadMemory()) ?? '').length > MEMORY_INLINE_CHARS
          const hidden = [...(layers.project ? fitProject(layers.project, budget).hidden : []), ...(globalLarge ? layers.global : [])]
          const recall = hidden.length > 0 ? recallNote(session, turn.request, hidden, Math.floor(budget / 2)) : null
          if (recall) notes.push(recall)
        }
      }
      const conflicts = memoryConflictNote(records, layerConflicts(layers), memory.surfaced)
      if (conflicts) notes.push(conflicts)
    } catch (err) {
      console.error('g11: tur hazırlığı başarısız:', err instanceof Error ? err.message : err)
    }
  }
  applyBlock(session.messages, turn.block)
  if (step > 0 && turn.request) {
    const drift = driftNote(session, turn.request)
    if (drift) notes.push(drift)
  }
  // Skilli model seçer (use_skill, arayüzde "Skill çalıştırıldı"): gövdeler zorla girince model konudan sapıyor ve bağlam şişiyordu.
  // Bu notlar kullanıcıya bildirim olarak gösterilmez ve dökümde görünmez (<system-reminder>).
  const quiet = [step === 0 && userText !== null && turn.request ? skillHintNote(session, turn.request) : null, skillPersistenceNote(session, step, userText)].filter((n): n is string => n !== null)
  if (quiet.length > 0) session.messages.push({ role: 'user', content: `<system-reminder>${quiet.join('\n\n')}</system-reminder>` })
  if (notes.length === 0) return
  const text = notes.join('\n\n')
  session.messages.push({ role: 'user', content: text })
  io.notice?.(text)
}

const MAX_SKILL_HINTS = 3

/** İsteğe uyabilecek skillerin adı ve tek satırı. Gövde girmez; uyuyorsa model use_skill ile kendisi yükler. */
function skillHintNote(session: Session, request: string): string | null {
  const hits = matchSkills(request, session.activeSkills ?? new Set(), MAX_SKILL_HINTS)
  if (hits.length === 0) return null
  const list = hits.map(({ skill }) => `- ${skill.name}: ${skill.description.slice(0, 120)}`).join('\n')
  return `Skills that may fit this request (keyword match, often wrong). If one clearly fits the actual job, call use_skill with its name before working; otherwise ignore this and do not mention it:\n${list}`
}

/** Modelin use_skill ile yüklediği skiller: sıkıştırma gövdeleri özete düşürdüyse yeniden verilir, yeni turda hatırlatılır. */
function skillPersistenceNote(session: Session, step: number, userText: string | null): string | null {
  const active = session.activeSkills
  if (!active || active.size === 0) return null
  if (session.skillsReinject) {
    session.skillsReinject = false
    const matches = [...active].flatMap((n) => {
      const skill = getSkill(n)
      return skill ? [{ skill, score: 0 }] : []
    })
    return autoSkillSection(matches.slice(0, 3))?.replace('# ACTIVE SKILLS (auto-selected for this request)', '# ACTIVE SKILLS (re-supplied after context compaction — still in force)') ?? null
  }
  if (step === 0 && userText !== null) return `Skills you loaded in this session: ${[...active].join(', ')}. Keep following their method; if their text is no longer in context, reload with use_skill(name).`
  return null
}
