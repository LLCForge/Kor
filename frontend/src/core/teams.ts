// [cdx] Takımlar: rolleri olan ajan ekipleri. Bir takım, sırayla çalışan rollerden oluşur (planlayıcı → geliştirici →
// gözden geçiren gibi); her rol bir ajan tipiyle (agents.ts) çalışır ve önceki rollerin raporunu bağlam olarak alır.
// Tanımlar: yerleşik (BUILTIN_TEAMS), kullanıcı (~/.kor/teams.json), proje (<proje>/.kor/teams.json; güven gerekir).
// Çalıştırma: run_team aracı (tools.ts) → ctx.spawnSubagent ile her rol bir alt ajan olarak sırayla koşar.
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { USER_CONFIG_DIR } from './settings.ts'
import { projectDotDir } from './layout-g28.ts'

export interface TeamRole {
  /** Rol adı (ör. "planlayıcı") */
  role: string
  /** Çalıştırılacak ajan tipi (agents.ts: general-purpose, Explore, Plan, code-reviewer, …) */
  agent: string
  /** Bu rolün görevi; {task} görevle, {previous} önceki rollerin raporuyla değiştirilir */
  instructions: string
}

export interface TeamDefinition {
  name: string
  description: string
  roles: TeamRole[]
  source: 'builtin' | 'user' | 'project'
}

const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/
export const teamRoleSchema = z.object({
  role: z.string().trim().min(1).max(40),
  agent: z.string().trim().min(1).max(60),
  instructions: z.string().trim().min(1).max(8_000),
})
export const teamFileSchema = z
  .array(
    z.object({
      name: z.string().regex(NAME_RE, 'Takım adı harf/rakamla başlamalı; harf, rakam, _ ve - (en fazla 40)'),
      description: z.string().trim().min(1).max(300),
      roles: z.array(teamRoleSchema).min(1).max(12),
    }),
  )
  .max(50)

export const BUILTIN_TEAMS: readonly TeamDefinition[] = [
  {
    name: 'ozellik',
    description: 'Yeni özellik: planla → uygula → gözden geçir.',
    source: 'builtin',
    roles: [
      { role: 'planlayıcı', agent: 'Plan', instructions: 'Görev: {task}\n\nKod tabanını incele ve dosya dosya uygulama planı çıkar: değişecek dosyalar, yeni modüller, testler, riskler. Kod yazma.' },
      { role: 'geliştirici', agent: 'general-purpose', instructions: 'Görev: {task}\n\nÖnceki rolün planı:\n{previous}\n\nPlanı uygula: kodu yaz, testleri ekle/çalıştır, derlemeyi doğrula. Sonunda yapılan değişiklikleri dosya listesiyle özetle.' },
      { role: 'gözden geçiren', agent: 'code-reviewer', instructions: 'Görev: {task}\n\nGeliştiricinin raporu:\n{previous}\n\nDeğişen dosyaları oku; doğruluk, güvenlik ve eksik test açısından incele. Bulduğun sorunları önem sırasıyla listele; kritik olanları düzelt.' },
    ],
  },
  {
    name: 'hata',
    description: 'Hata düzeltme: keşfet → düzelt → doğrula.',
    source: 'builtin',
    roles: [
      { role: 'kaşif', agent: 'Explore', instructions: 'Hata: {task}\n\nHatanın kök nedenini bul: ilgili dosyalar, çağrı zinciri, yeniden üretme adımları. Kod değiştirme; bulgularını dosya:satır referanslarıyla raporla.' },
      { role: 'geliştirici', agent: 'general-purpose', instructions: 'Hata: {task}\n\nKaşif raporu:\n{previous}\n\nKök nedeni düzelt, regresyon testi ekle ve testleri çalıştır. Yaptığın değişiklikleri özetle.' },
      { role: 'doğrulayıcı', agent: 'code-reviewer', instructions: 'Hata: {task}\n\nDüzeltme raporu:\n{previous}\n\nDüzeltmenin kök nedeni kapattığını ve yan etki yaratmadığını doğrula; eksik varsa tamamla.' },
    ],
  },
  {
    name: 'inceleme',
    description: 'Kod incelemesi: bağımsız inceleyici + sadeleştirici.',
    source: 'builtin',
    roles: [
      { role: 'inceleyici', agent: 'code-reviewer', instructions: 'İnceleme konusu: {task}\n\nDeğişiklikleri doğruluk, güvenlik, performans ve test kapsamı açısından incele; bulguları önem sırasıyla listele.' },
      { role: 'sadeleştirici', agent: 'code-simplifier', instructions: 'İnceleme konusu: {task}\n\nİnceleyici bulguları:\n{previous}\n\nDavranışı değiştirmeden kodu sadeleştir; inceleyicinin kritik bulgularını düzelt. Yaptıklarını özetle.' },
    ],
  },
]

let active: readonly TeamDefinition[] = BUILTIN_TEAMS
export function setTeams(teams: readonly TeamDefinition[]): void {
  active = teams
}
export function getTeams(): readonly TeamDefinition[] {
  return active
}
export function getTeam(name: string): TeamDefinition | undefined {
  const key = name.trim().toLowerCase()
  return active.find((t) => t.name.toLowerCase() === key)
}

async function loadFile(file: string, source: 'user' | 'project', warn: (m: string) => void): Promise<TeamDefinition[]> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    warn(`${file}: ${err instanceof Error ? err.message : String(err)}`)
    return []
  }
  try {
    return teamFileSchema.parse(JSON.parse(text)).map((t) => ({ ...t, source }))
  } catch (err) {
    warn(`${file}: ${err instanceof z.ZodError ? z.prettifyError(err) : err instanceof Error ? err.message : String(err)}`)
    return []
  }
}

/** Yerleşik + kullanıcı + (güvenilen) proje takımları; aynı ad sonradan gelenle ezilir. */
export async function loadTeams(root: string | null, trusted: boolean, warn: (m: string) => void = () => undefined): Promise<TeamDefinition[]> {
  const byName = new Map<string, TeamDefinition>()
  for (const t of BUILTIN_TEAMS) byName.set(t.name.toLowerCase(), t)
  for (const t of await loadFile(path.join(USER_CONFIG_DIR, 'teams.json'), 'user', warn)) byName.set(t.name.toLowerCase(), t)
  if (root && trusted) for (const t of await loadFile(path.join(projectDotDir(root), 'teams.json'), 'project', warn)) byName.set(t.name.toLowerCase(), t)
  return [...byName.values()]
}

/** Sistem istemi bölümü: takımlar ve rolleri (model run_team ile çalıştırır). */
export function teamCatalogSection(): string | null {
  const teams = getTeams()
  if (teams.length === 0) return null
  const lines = teams.map((t) => `- ${t.name}: ${t.description} (roles: ${t.roles.map((r) => `${r.role}→${r.agent}`).join(', ')})`)
  return `# TEAMS
Role-based agent teams that run their roles IN ORDER as subagents, each receiving the previous roles' reports. Use run_team when the user asks for a team ("takım"), a planned multi-role workflow, or a full feature/bugfix cycle with review:
${lines.join('\n')}`
}

/** Rol talimatını doldurur. */
export function renderRolePrompt(role: TeamRole, task: string, previous: string): string {
  return role.instructions.replace(/\{task\}/g, task).replace(/\{previous\}/g, previous || '(önceki rol yok)')
}

export interface TeamRunStep {
  role: string
  agent: string
  report: string
}

/**
 * Takımı sırayla çalıştırır: her rol önceki raporları bağlam olarak alır. spawn, run_team aracının ctx.spawnSubagent'ıdır.
 * onStep her rol bitince çağrılır (arayüz ilerleme için).
 */
export async function runTeam(
  team: TeamDefinition,
  task: string,
  spawn: (req: { description: string; prompt: string; subagent_type: string }) => Promise<string>,
  onStep?: (step: TeamRunStep, index: number) => void,
): Promise<TeamRunStep[]> {
  const steps: TeamRunStep[] = []
  for (const [i, role] of team.roles.entries()) {
    const previous = steps.map((s) => `## ${s.role} (${s.agent})\n${s.report}`).join('\n\n')
    const report = await spawn({
      description: `${team.name}/${role.role}`.slice(0, 80),
      prompt: renderRolePrompt(role, task, previous),
      subagent_type: role.agent,
    })
    const step = { role: role.role, agent: role.agent, report }
    steps.push(step)
    onStep?.(step, i)
  }
  return steps
}

export function formatTeamResult(team: TeamDefinition, steps: readonly TeamRunStep[]): string {
  return `Takım "${team.name}" tamamlandı (${steps.length} rol):\n\n${steps.map((s, i) => `### ${i + 1}. ${s.role} — ${s.agent}\n${s.report}`).join('\n\n')}`
}
