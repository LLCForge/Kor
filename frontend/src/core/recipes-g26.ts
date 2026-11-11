// [g26] Proje tarifleri. Zamanlama uygulama açıkken dakikalık taramayla olur; işletim sistemi görevi kurulmaz.
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { projectDotDir } from './layout-g28.ts' // [g28]

const NAME = /^[A-Za-z0-9][A-Za-z0-9_-]{0,40}$/

export interface Recipe {
  name: string
  cron: string
  tools: string[]
  prompt: string
}

const recipeSchema = z.object({
  name: z.string().regex(NAME),
  cron: z.string().min(9).max(80),
  tools: z.array(z.string().regex(/^[a-z0-9_]{1,60}$/)).min(1).max(30),
  prompt: z.string().min(1).max(10_000),
})

function fieldValues(field: string, min: number, max: number): number[] {
  if (field === '*') return Array.from({ length: max - min + 1 }, (_, i) => i + min)
  const step = /^\*\/(\d+)$/.exec(field)
  if (step?.[1]) {
    const n = Number(step[1])
    if (n < 1) throw new Error(`Geçersiz cron adımı: ${field}`)
    const values: number[] = []
    for (let value = min; value <= max; value += n) values.push(value)
    return values
  }
  const values: number[] = []
  for (const part of field.split(',')) {
    const range = /^(\d+)-(\d+)$/.exec(part)
    if (range?.[1] && range[2]) {
      const from = Number(range[1])
      const to = Number(range[2])
      if (from > to) throw new Error(`Geçersiz cron aralığı: ${part}`)
      for (let value = from; value <= to; value++) values.push(value)
    } else {
      if (!/^\d+$/.test(part)) throw new Error(`Geçersiz cron alanı: ${field}`)
      values.push(Number(part))
    }
  }
  if (values.some((value) => value < min || value > max)) throw new Error(`Cron alanı sınır dışında: ${field}`)
  return values
}

export function cronMatches(expr: string, when: Date): boolean {
  const fields = expr.trim().split(/\s+/)
  if (fields.length !== 5) throw new Error('Cron beş alandır: dakika saat gün ay haftanın günü')
  const minute = fieldValues(fields[0] ?? '', 0, 59)
  const hour = fieldValues(fields[1] ?? '', 0, 23)
  const day = fieldValues(fields[2] ?? '', 1, 31)
  const month = fieldValues(fields[3] ?? '', 1, 12)
  const week = fieldValues(fields[4] ?? '', 0, 7).map((value) => (value === 7 ? 0 : value))
  return (
    minute.includes(when.getMinutes()) &&
    hour.includes(when.getHours()) &&
    day.includes(when.getDate()) &&
    month.includes(when.getMonth() + 1) &&
    week.includes(when.getDay())
  )
}

export function parseRecipe(text: string): Recipe {
  const lines = text.split(/\r?\n/)
  const fields: Record<string, string> = {}
  let promptBlock = false
  const promptLines: string[] = []
  for (const line of lines) {
    if (promptBlock) {
      promptLines.push(line)
      continue
    }
    if (/^prompt:\s*\|\s*$/.test(line)) {
      promptBlock = true
      continue
    }
    const match = /^([A-Za-z]+):\s*(.*)$/.exec(line)
    if (match?.[1]) fields[match[1]] = match[2] ?? ''
  }
  const tools = (fields.tools ?? '')
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean)
  const prompt = (promptBlock ? promptLines.join('\n') : fields.prompt ?? '').trim()
  const recipe = recipeSchema.parse({ name: (fields.name ?? '').trim(), cron: (fields.cron ?? '').replace(/^["']|["']$/g, ''), tools, prompt })
  cronMatches(recipe.cron, new Date())
  return recipe
}

function recipeDir(root: string): string {
  return path.join(projectDotDir(root), 'recipes')
}

export async function loadRecipes(root: string): Promise<Recipe[]> {
  let names: string[]
  try {
    names = await readdir(recipeDir(root))
  } catch {
    return []
  }
  const recipes: Recipe[] = []
  for (const name of names.filter((item) => item.endsWith('.yaml') || item.endsWith('.yml')).sort()) {
    try {
      recipes.push(parseRecipe(await readFile(path.join(recipeDir(root), name), 'utf8')))
    } catch (err) {
      console.warn(`Tarif atlandı (${name}):`, err instanceof Error ? err.message : err)
    }
  }
  return recipes
}

async function stateOf(root: string): Promise<Record<string, number>> {
  try {
    const parsed = z.record(z.string(), z.number()).safeParse(JSON.parse(await readFile(path.join(projectDotDir(root), 'recipe-state.json'), 'utf8')))
    return parsed.success ? parsed.data : {}
  } catch {
    return {}
  }
}

/** Bu dakikada cron tutan ve henüz çalışmamış tarifler. Kayıt, çalışmadan önce yazılır. */
export async function claimDueRecipes(root: string, now: number = Date.now()): Promise<Recipe[]> {
  const recipes = await loadRecipes(root)
  const state = await stateOf(root)
  const minute = new Date(now)
  minute.setSeconds(0, 0)
  const due: Recipe[] = []
  for (const recipe of recipes) {
    const last = state[recipe.name] ?? 0
    if (last >= minute.getTime()) continue
    if (!cronMatches(recipe.cron, minute)) continue
    state[recipe.name] = now
    due.push(recipe)
  }
  if (due.length === 0) return []
  await mkdir(projectDotDir(root), { recursive: true })
  await writeFile(path.join(projectDotDir(root), 'recipe-state.json'), JSON.stringify(state), { mode: 0o600 })
  return due
}

export async function writeRecipe(root: string, recipe: Recipe): Promise<string> {
  const dir = recipeDir(root)
  await mkdir(dir, { recursive: true })
  const file = path.join(dir, `${recipe.name}.yaml`)
  const body = `name: ${recipe.name}\ncron: "${recipe.cron}"\ntools: ${recipe.tools.join(', ')}\nprompt: |\n${recipe.prompt}\n`
  await writeFile(file, body, { mode: 0o600 })
  return file
}
