// [g3] Sistem istemine eklenen bölümler: etkileşim araçları politikası ve seçili çıktı stili.
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { isOutputStyle, OUTPUT_STYLES, type OutputStyle } from './output-styles.ts'
import { USER_CONFIG_DIR } from './settings.ts'

export const OUTPUT_STYLE_FILE = path.join(USER_CONFIG_DIR, 'output-style.json')

export const INTERACTION_POLICY = `# INTERACTION TOOLS POLICY
- todo_write: for any task with more than one step, create the list together with your first tool call (see "How to run a task"). CRITICAL: the moment you finish a step, your very next action is a todo_write call that re-sends the FULL list with that step marked "completed" and the next step set to "in_progress" (exactly one "in_progress" at a time). Update it as you go, step by step — never only at the end, and never leave a finished step as "pending" or "in_progress". A step is not done until you have marked it completed. Skip the list only for trivial one-step tasks.
- mark_chapter: when the work shifts to a meaningfully different phase (exploration -> implementation -> verification, or the user pivots to an unrelated request), call mark_chapter with a short Turkish noun-phrase title (2-5 words) and an optional one-line summary. Use it sparingly: a long session typically has 3-8 chapters. Do not open a chapter for the very first message or for every tool call.
- ask_user: keep going on your own whenever a reasonable assumption exists (state it and continue). Ask ONLY when the choice materially changes the outcome and you genuinely cannot pick: two real paths, a destructive/irreversible step, a business/design preference, or missing information only the user has. Then give 2-6 SHORT options in the user's own vocabulary, best option first, each with a one-line consequence; mark your recommendation and put it first. Never ask what a default can answer, never ask twice in a row, and never ask just to confirm a plan you already explained.
- exit_plan_mode: in plan mode, editing files and running commands is blocked. Investigate with read-only tools, then call exit_plan_mode once with the complete plan in Markdown (steps, files to change, verification). Do not try to edit before approval. If the user sends feedback instead of approving, revise the plan and call exit_plan_mode again. If the user says "just do it", treat that as approval and get on with it.
- Plan mode is not a sandbox, it is a pause: what you can do in one turn you should plan and then do in one go, not drip-feed across approvals.
- suggest_next: when a turn that changed files or did multi-step work ends, call suggest_next as your last action with a 2-3 sentence recap (what is now true, what is not done or not verified). Add one question and up to 3 short reply options only when the next move is the user's decision. Do not repeat the recap in plain text.`

/** Dosya yoksa veya bozuksa varsayılan stil; bozuk ayar sohbeti durdurmamalı. */
export async function loadOutputStyle(file: string = OUTPUT_STYLE_FILE): Promise<OutputStyle> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return 'default'
    throw err
  }
  try {
    const parsed: unknown = JSON.parse(text)
    const style = typeof parsed === 'object' && parsed !== null && 'style' in parsed ? parsed.style : undefined
    return isOutputStyle(style) ? style : 'default'
  } catch {
    console.warn(`${file}: geçersiz JSON, varsayılan çıktı stili kullanılıyor`)
    return 'default'
  }
}

export async function saveOutputStyle(style: OutputStyle, file: string = OUTPUT_STYLE_FILE): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({ style })}\n`, { encoding: 'utf8', mode: 0o600 })
}

export function outputStyleSection(style: OutputStyle): string | null {
  return OUTPUT_STYLES.find((s) => s.id === style)?.prompt ?? null
}

export async function g3PromptSections(file?: string): Promise<string[]> {
  const style = outputStyleSection(await loadOutputStyle(file))
  return style ? [INTERACTION_POLICY, style] : [INTERACTION_POLICY]
}
