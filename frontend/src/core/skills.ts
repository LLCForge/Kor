// Skiller (Claude uyumlu SKILL.md): uzman iş akışları, talep üzerine yüklenir. Her skill, içinde SKILL.md olan bir
// klasördür (frontmatter: name, description; gövde = talimatlar). Sistem isteminde yalnızca ad+açıklama listelenir;
// model use_skill aracıyla tam gövdeyi ve klasördeki ek dosyaları yükler. Böylece bağlam dolmadan derin uzmanlık.
// Dizinler — proje (yalnızca klasör güveniliyse): <root>/.claude/skills, <root>/.kor/skills.
// Kullanıcı: ~/.claude/skills ve <USER_CONFIG_DIR>/skills.
// [g31] Gömülü skiller (uygulamayla gelir, depo talimatı değildir): src/core/builtin-skills → dist/builtin-skills.
// Gömülü ajanlarla aynı kural: her zaman yüklenir; aynı adlı kullanıcı/proje skili gömülü olanı ezer.
import { readdir, readFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url' // [g31]
import { enabledPluginDirs } from './plugins.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { projectDotDir } from './layout-g28.ts' // [g28]

const MAX_BODY = 50_000
const MAX_FILES = 100
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/

export interface Skill {
  name: string
  description: string
  body: string
  /** SKILL.md'nin bulunduğu klasör (ek dosyalar buradan, yol hapsiyle okunur). */
  dir: string
  source: 'builtin' | 'user' | 'project' // [g31] builtin: uygulamayla gelen skill
  /** [cdx] Eklentiden geliyorsa eklenti adı */
  origin?: string
}

export function parseSkillFile(text: string, dirName: string, dir: string, source: Skill['source']): Skill {
  const fm = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text)
  if (!fm) throw new Error('SKILL.md --- frontmatter içermeli (name, description)')
  const fields: Record<string, string> = {}
  for (const line of (fm[1] ?? '').split(/\r?\n/)) {
    const kv = /^([A-Za-z-]+)\s*:\s*(.*)$/.exec(line.trim())
    if (kv?.[1]) fields[kv[1].toLowerCase()] = (kv[2] ?? '').replace(/^["']|["']$/g, '').trim()
  }
  const name = fields.name || dirName
  if (!NAME_RE.test(name)) throw new Error('skill adı harf/rakamla başlamalı (harf, rakam, _ , -)')
  const description = (fields.description ?? '').slice(0, 1024)
  if (!description) throw new Error('SKILL.md içinde description gerekli')
  const body = (fm[2] ?? '').trim()
  if (!body) throw new Error('SKILL.md gövdesi boş olamaz')
  return { name, description, body: body.slice(0, MAX_BODY), dir, source }
}

let active: readonly Skill[] = []
export function setSkills(skills: readonly Skill[]): void {
  active = skills
}
export function getSkills(): readonly Skill[] {
  return active
}
export function getSkill(name: string): Skill | undefined {
  return active.find((s) => s.name === name)
}

/** Sistem istemine giren kısa katalog (yalnızca ad+açıklama). Skill yoksa null. */
// Katalog her model isteğine eklenir: binlerce skill (ör. ~/.claude/skills) ~60 bin token tutup bağlamı dolduruyordu.
export const MAX_CATALOG_CHARS = 12_000
const MAX_CATALOG_DESC = 140
const SOURCE_RANK: Record<Skill['source'], number> = { project: 0, builtin: 1, user: 2 }

export function skillCatalogSection(): string | null {
  const skills = getSkills()
  if (skills.length === 0) return null
  // Projeye özgü ve gömülü skiller önce: bütçe dolarsa kesilen, genel kullanıcı kütüphanesi olur.
  const ordered = [...skills].sort((a, b) => SOURCE_RANK[a.source] - SOURCE_RANK[b.source])
  const lines: string[] = []
  let size = 0
  for (const s of ordered) {
    const desc = s.description.length > MAX_CATALOG_DESC ? `${s.description.slice(0, MAX_CATALOG_DESC - 1)}…` : s.description
    const line = `- ${s.name}: ${desc}`
    if (size + line.length + 1 > MAX_CATALOG_CHARS) break
    lines.push(line)
    size += line.length + 1
  }
  const hidden = skills.length - lines.length
  const more = hidden > 0 ? `\n(+${hidden} more skills not listed here. To see ALL of them call list_skills (paginated; query/source filters); to find one by topic call use_skill with a keyword. Never scan the filesystem for skills.)` : ''
  return `# SKILLS
Expert workflows, loaded only when you choose:
- The job clearly matches a skill → call use_skill with its name BEFORE working, once, then follow it.
- Not sure which one → use_skill with a keyword instead of an exact name; it returns the closest matches.
- No clear match → work without one. Load at most one or two skills for a task; a skill that only shares a word with the request is not a match.\n${lines.join('\n')}${more}`
}

/** Ada ve açıklamaya göre en yakın skiller (use_skill tam ad bulamadığında). */
export function findSkills(query: string, limit = 15): Skill[] {
  const terms = query.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter((t) => t.length >= 2)
  if (terms.length === 0) return []
  const scored = getSkills()
    .map((s) => {
      const name = s.name.toLowerCase()
      const desc = s.description.toLowerCase()
      const score = terms.reduce((acc, t) => acc + (name.includes(t) ? 3 : 0) + (desc.includes(t) ? 1 : 0), 0)
      return { s, score }
    })
    .filter((x) => x.score > 0)
  scored.sort((a, b) => b.score - a.score || a.s.name.localeCompare(b.s.name))
  return scored.slice(0, limit).map((x) => x.s)
}

/** Skill klasöründeki dosyaları (SKILL.md hariç) göreli yollarla, sınırlı sayıda listeler. */
export async function listSkillFiles(dir: string): Promise<string[]> {
  const out: string[] = []
  const walk = async (rel: string): Promise<void> => {
    if (out.length >= MAX_FILES) return
    let entries
    try {
      entries = await readdir(path.join(dir, rel), { withFileTypes: true })
    } catch {
      return
    }
    for (const e of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      if (out.length >= MAX_FILES) return
      const r = rel ? `${rel}/${e.name}` : e.name
      if (e.isDirectory()) await walk(r)
      else if (r !== 'SKILL.md') out.push(r)
    }
  }
  await walk('')
  return out
}

async function loadFrom(dir: string, source: Skill['source'], byName: Map<string, Skill>, warn: (msg: string) => void, origin?: string): Promise<void> {
  let entries
  try {
    entries = await readdir(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    if (!e.isDirectory()) continue
    const skillDir = path.join(dir, e.name)
    try {
      const text = await readFile(path.join(skillDir, 'SKILL.md'), 'utf8')
      const skill = parseSkillFile(text, e.name, skillDir, source)
      byName.set(skill.name, origin ? { ...skill, origin } : skill) // sonra gelen (proje) kazanır
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'ENOENT') warn(`${path.join(skillDir, 'SKILL.md')}: ${err instanceof Error ? err.message : String(err)}`)
    }
  }
}

/**
 * [g31] Uygulamayla gelen skill klasörleri. Kaynak çalıştırmada modülün yanında, pakette dist yanında.
 * Gömülü ajanlarla aynı yol mantığı (agents.ts embeddedAgentDirs); import.meta.url tanımsızsa atlanır.
 */
function embeddedSkillDirs(): string[] {
  let here: string
  try {
    here = path.dirname(fileURLToPath(import.meta.url))
  } catch {
    return []
  }
  const dirs = [path.join(here, 'builtin-skills'), path.join(here, 'src', 'core', 'builtin-skills')]
  const seen = new Set<string>()
  return dirs.filter((d) => {
    const key = process.platform === 'win32' ? path.resolve(d).toLowerCase() : path.resolve(d)
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

/** homeDir enjekte edilebilir (test); varsayılan gerçek ev dizini (~/.claude/skills okunur). */
export async function loadSkills(root: string | null, trusted: boolean, warn: (msg: string) => void = () => undefined, homeDir: string = os.homedir()): Promise<Skill[]> {
  const byName = new Map<string, Skill>()
  // [g31] Gömülü skiller güven kapısından bağımsız yüklenir; proje/kullanıcı skili aynı adı ezebilir.
  for (const dir of embeddedSkillDirs()) await loadFrom(dir, 'builtin', byName, warn)
  // Ajan ekosistemi klasörleri: Claude Code (~/.claude/skills), Codex/Gemini/Cursor ortak (~/.agents/skills — rea setup buraya kurar),
  // Codex (~/.codex/skills) ve uygulamanın kendi klasörü.
  await loadFrom(path.join(homeDir, '.claude', 'skills'), 'user', byName, warn)
  await loadFrom(path.join(homeDir, '.agents', 'skills'), 'user', byName, warn)
  await loadFrom(path.join(homeDir, '.codex', 'skills'), 'user', byName, warn)
  await loadFrom(path.join(USER_CONFIG_DIR, 'skills'), 'user', byName, warn)
  if (root && trusted) {
    await loadFrom(path.join(root, '.claude', 'skills'), 'project', byName, warn)
    await loadFrom(path.join(root, '.agents', 'skills'), 'project', byName, warn)
    await loadFrom(path.join(projectDotDir(root), 'skills'), 'project', byName, warn)
  }
  // Eklenti skilleri: skills/, .agents/skills/, .claude/skills/ ve manifest 'skills' yolu (rea: .agents/skills).
  for (const p of await enabledPluginDirs(root, trusted)) for (const d of p.skillDirs) await loadFrom(d, p.source, byName, warn, p.name)
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name))
}
