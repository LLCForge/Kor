// [g3] Etkileşimli araçların ortak şemaları ve sonuç biçimleri.
// Saf modül (yalnızca zod): hem çekirdek hem arayüz (summary JSON'unu çözmek için) kullanır.
import { z } from 'zod'
import type { PermissionMode } from './permissions.ts'

export const chapterArgs = z.object({
  title: z.string().trim().min(1).max(60).describe('Bölüm başlığı: 2-5 kelimelik isim öbeği (Türkçe)'),
  summary: z.string().trim().max(200).optional().describe('İsteğe bağlı tek satırlık özet'),
})
export type ChapterArgs = z.infer<typeof chapterArgs>

export const TODO_STATUSES = ['pending', 'in_progress', 'completed'] as const
export type TodoStatus = (typeof TODO_STATUSES)[number]

export const todoArgs = z.object({
  items: z
    .array(
      z.object({
        content: z.string().trim().min(1).max(200).describe('Kısa, emir kipinde görev'),
        status: z.enum(TODO_STATUSES),
      }),
    )
    .min(1)
    .max(50)
    .describe('Listenin TAMAMI; her çağrıda bütün görevleri gönder'),
})
export type TodoItem = z.infer<typeof todoArgs>['items'][number]

export const suggestArgs = z.object({
  recap: z.string().trim().min(1).max(800).describe('Ne değişti / nerede kaldık: 2-3 kısa cümle'),
  question: z.string().trim().min(1).max(200).optional().describe('Kullanıcının vermesi gereken tek karar, varsa'),
  options: z
    .array(
      z.object({
        label: z.string().trim().min(1).max(40).describe('Düğme metni, 1-4 kelime'),
        message: z.string().trim().min(1).max(1000).describe('Düğmeye basılınca kullanıcı adına gönderilecek mesaj'),
      }),
    )
    .max(3)
    .optional(),
})
export type SuggestArgs = z.infer<typeof suggestArgs>

const askArgsSimple = z
  .object({
    question: z.string().trim().min(1).max(500),
    options: z
      .array(
        z.object({
          label: z.string().trim().min(1).max(80),
          description: z.string().trim().max(300).optional(),
        }),
      )
      .min(2)
      .max(6),
    multiSelect: z.boolean().optional().describe('true ise kullanıcı birden fazla seçenek işaretleyebilir'),
  })
  .refine((a) => new Set(a.options.map((o) => o.label)).size === a.options.length, 'Seçenek etiketleri benzersiz olmalı')

const formQuestion = z
  .object({
    question: z.string().trim().min(1).max(500),
    type: z.enum(['single_select', 'multi_select', 'rank_priorities']),
    options: z.array(z.string().trim().min(1).max(80)).min(2).max(4),
  })
  .refine((q) => new Set(q.options).size === q.options.length, 'Seçenekler benzersiz olmalı')

const formAsk = z.object({
  questions: z.array(formQuestion).min(1).max(3),
})

export const askArgs = z.union([askArgsSimple, formAsk])
export type AskArgs = z.infer<typeof askArgs>
export type SimpleAsk = z.infer<typeof askArgsSimple>
export type FormAsk = z.infer<typeof formAsk>

export function isAskForm(args: AskArgs): args is FormAsk {
  return 'questions' in args
}

export const planArgs = z.object({
  plan: z.string().trim().min(1).max(50_000).describe('Uygulanacak plan, Markdown'),
})

/** Arayüzün araç kartını çizmek için çözdüğü yük; araçların summary alanı bu nesnenin JSON'udur. */
export type InteractivePayload =
  | { tool: 'mark_chapter'; args: ChapterArgs }
  | { tool: 'todo_write'; args: z.infer<typeof todoArgs> }
  | { tool: 'suggest_next'; args: SuggestArgs }
  | { tool: 'ask_user'; args: AskArgs }
  | { tool: 'exit_plan_mode'; args: z.infer<typeof planArgs> }

const PAYLOAD_SCHEMAS = {
  mark_chapter: chapterArgs,
  todo_write: todoArgs,
  suggest_next: suggestArgs,
  ask_user: askArgs,
  exit_plan_mode: planArgs,
} as const

export const INTERACTIVE_TOOL_NAMES = Object.keys(PAYLOAD_SCHEMAS) as ReadonlyArray<InteractivePayload['tool']>

/** Araç summary'sindeki JSON'u doğrular; model çıktısından geldiği için güvenilmez kabul edilir. */
export function decodeInteractive(name: string, summary: string): InteractivePayload | null {
  if (!(name in PAYLOAD_SCHEMAS)) return null
  let raw: unknown
  try {
    raw = JSON.parse(summary)
  } catch {
    return null
  }
  const parsed = PAYLOAD_SCHEMAS[name as InteractivePayload['tool']].safeParse(raw)
  return parsed.success ? ({ tool: name, args: parsed.data } as InteractivePayload) : null
}

/** Arayüzden ana sürece gelen cevap (IPC sınırında doğrulanır). */
export const interactAnswer = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('ask'),
    selected: z.array(z.string().max(80)).max(6),
    other: z.string().trim().max(2000).optional(),
    answers: z
      .array(
        z.object({
          selected: z.array(z.string().max(80)).max(4),
          other: z.string().trim().max(2000).optional(),
          ranked: z.array(z.string().max(80)).max(4).optional(),
          skipped: z.boolean().optional(),
        }),
      )
      .max(3)
      .optional(),
  }),
  z.object({ kind: z.literal('planApprove'), mode: z.enum(['auto', 'default']) }),
  z.object({ kind: z.literal('planRevise'), feedback: z.string().trim().min(1).max(5000) }),
  z.object({ kind: z.literal('draft'), confirmed: z.boolean() }),
])
export type InteractAnswer = z.infer<typeof interactAnswer>

/** Etkileşimli araçların kullanıcıyı beklediği köprü; yoksa (CLI, alt ajan) araç düz metne döner. */
export interface InteractBridge {
  ask(callId: string, args: AskArgs, signal: AbortSignal): Promise<string>
  exitPlan(callId: string, plan: string, signal: AbortSignal): Promise<string>
  /** Etkinlik taslağını onaylatır. Yoksa araç kaydetmez. */
  confirmDraft?(callId: string, signal: AbortSignal): Promise<boolean>
}

/** Cevabı soruya karşı doğrular ve modelin göreceği metne çevirir. Geçersizse hata fırlatır. */
export function formatAskAnswer(args: Pick<SimpleAsk, 'options' | 'multiSelect'>, answer: Extract<InteractAnswer, { kind: 'ask' }>): string {
  const labels = new Set(args.options.map((o) => o.label))
  const selected = [...new Set(answer.selected)]
  if (selected.some((s) => !labels.has(s))) throw new Error('Geçersiz seçenek')
  if (!args.multiSelect && selected.length > 1) throw new Error('Bu soru tek seçimlidir')
  const other = answer.other?.trim() ?? ''
  if (selected.length === 0 && !other) throw new Error('Bir seçenek işaretle veya cevabını yaz')
  const lines: string[] = []
  if (selected.length > 0) lines.push(`Kullanıcının seçimi: ${selected.map((s) => `"${s}"`).join(', ')}`)
  if (other) lines.push(selected.length > 0 ? `Ek not: ${other}` : `Kullanıcının cevabı (serbest metin): ${other}`)
  return lines.join('\n')
}

/** Çok sorulu formun cevabı. Atla, serbest metin ve sıralama geçerlidir. */
export function formatFormAnswer(args: FormAsk, answer: Extract<InteractAnswer, { kind: 'ask' }>): string {
  const rows = answer.answers
  if (!rows || rows.length !== args.questions.length) throw new Error('Her soru cevaplanmalı')
  return args.questions
    .map((q, i) => {
      const row = rows[i]
      if (!row) throw new Error('Her soru cevaplanmalı')
      if (row.skipped) return `${i + 1}. ${q.question}\nAtlandı`
      if (q.type === 'rank_priorities') {
        const ranked = row.ranked ?? []
        const same = ranked.length === q.options.length && new Set(ranked).size === ranked.length && ranked.every((s) => q.options.includes(s))
        if (!same) throw new Error('Sıralama seçeneklerin tamamını içermeli')
        return `${i + 1}. ${q.question}\nSıralama: ${ranked.map((s) => `"${s}"`).join(' > ')}`
      }
      const selected = [...new Set(row.selected)]
      if (selected.some((s) => !q.options.includes(s))) throw new Error('Geçersiz seçenek')
      if (q.type !== 'multi_select' && selected.length > 1) throw new Error('Bu soru tek seçimlidir')
      const other = row.other?.trim() ?? ''
      if (selected.length === 0 && !other) throw new Error('Bir seçenek işaretle, kendi cevabını yaz veya atla')
      const lines = [`${i + 1}. ${q.question}`]
      if (selected.length > 0) lines.push(`Seçim: ${selected.map((s) => `"${s}"`).join(', ')}`)
      if (other) lines.push(`Başka bir şey: ${other}`)
      return lines.join('\n')
    })
    .join('\n\n')
}

const MODE_LABELS: Partial<Record<PermissionMode, string>> = { auto: 'Otomatik', default: 'Manuel' }

export function formatPlanDecision(answer: Extract<InteractAnswer, { kind: 'planApprove' | 'planRevise' }>): string {
  if (answer.kind === 'planApprove') {
    return `Kullanıcı planı onayladı. İzin modu "${MODE_LABELS[answer.mode] ?? answer.mode}" oldu; plan modundan çıktın. Şimdi planı adım adım uygula.`
  }
  return `Kullanıcı planı onaylamadı. Geri bildirimi:\n${answer.feedback}\n\nPlan modunda kal; planı bu geri bildirime göre güncelle ve exit_plan_mode ile yeniden sun.`
}

/** "2/5 tamamlandı" gibi ilerleme özeti. */
export function todoProgress(items: readonly TodoItem[]): { done: number; total: number; active: TodoItem | undefined } {
  return {
    done: items.filter((i) => i.status === 'completed').length,
    total: items.length,
    active: items.find((i) => i.status === 'in_progress'),
  }
}
