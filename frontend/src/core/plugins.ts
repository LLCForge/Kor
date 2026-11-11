// Eklenti paketleri: komut, skill, ajan ve araçları tek klasörde toplar. Bir eklenti, içinde plugin.json (ya da
// .claude-plugin/plugin.json) ve commands/ skills/ agents/ tools/ alt klasörleri olan bir dizindir. Etkinleştirilen
// eklentilerin alt klasörleri, mevcut yükleyicilerce standart dizinlerle aynı biçimde (Claude/Cursor uyumlu) taranır.
// Yer: <USER_CONFIG_DIR>/plugins/<ad> (kullanıcı) ve <root>/.kor/plugins/<ad> (yalnızca klasör güveniliyse).
import { existsSync } from 'node:fs'
import { readdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { USER_CONFIG_DIR } from './settings.ts'
import { projectDotDir } from './layout-g28.ts' // [g28]

const PLUGINS_STATE_FILE = 'plugins.json'

export interface PluginManifest {
  name: string
  version: string | null
  description: string
  author: string | null
  /** plugin.json 'skills' alanı (yol ya da yol listesi) */
  skills?: string | string[]
}

export interface PluginContributions {
  commands: number
  skills: number
  agents: number
  tools: number
}

export interface DiscoveredPlugin {
  name: string
  /** Klon/klasör adı (manifest adından farklı olabilir; güncelle/kaldır bununla yapılır). */
  dirName: string
  dir: string
  source: 'user' | 'project'
  manifest: PluginManifest
  enabled: boolean
  contributions: PluginContributions
  /** .git içeriyor: git kurucusuyla kurulmuş (güncellenebilir/kaldırılabilir). */
  managed: boolean
}

const manifestSchema = z.object({
  name: z.string().max(100).optional(),
  version: z.string().max(40).optional(),
  description: z.string().max(1000).optional(),
  author: z.unknown().optional(),
  /** Claude Code plugin.json: skill klasörü ya da listesi ("./skills"); rea gibi eklentiler .agents/skills de kullanır */
  skills: z.union([z.string().max(300), z.array(z.string().max(300)).max(20)]).optional(),
})

/** Eklentinin skill klasörleri: manifest 'skills' + standart yerler (skills/, .agents/skills/, .claude/skills/). Tekil ve var olanlar. */
export function pluginSkillDirs(dir: string, manifestSkills?: string | string[]): string[] {
  const declared = (Array.isArray(manifestSkills) ? manifestSkills : manifestSkills ? [manifestSkills] : []).map((p) => path.resolve(dir, p))
  const standard = ['skills', path.join('.agents', 'skills'), path.join('.claude', 'skills')].map((p) => path.join(dir, p))
  const out: string[] = []
  for (const d of [...declared, ...standard]) {
    const rel = path.relative(dir, d)
    if (rel.startsWith('..') || path.isAbsolute(rel)) continue // eklenti klasörü dışına çıkma
    if (!out.includes(d) && existsSync(d)) out.push(d)
  }
  return out
}

const stateSchema = z.object({ disabled: z.array(z.string().max(100)).max(500).default([]) })

async function loadDisabled(): Promise<Set<string>> {
  try {
    const parsed = stateSchema.safeParse(JSON.parse(await readFile(path.join(USER_CONFIG_DIR, PLUGINS_STATE_FILE), 'utf8')))
    return new Set(parsed.success ? parsed.data.disabled : [])
  } catch {
    return new Set()
  }
}

/** Eklentiyi etkinleştirir/pasifleştirir (pasif listesi plugins.json'da tutulur; varsayılan etkin). */
export async function setPluginEnabled(name: string, enabled: boolean): Promise<void> {
  const disabled = await loadDisabled()
  if (enabled) disabled.delete(name)
  else disabled.add(name)
  await writeFile(path.join(USER_CONFIG_DIR, PLUGINS_STATE_FILE), JSON.stringify({ disabled: [...disabled] }, null, 2), { mode: 0o600 })
}

async function readManifest(dir: string): Promise<PluginManifest | null> {
  for (const rel of ['plugin.json', path.join('.claude-plugin', 'plugin.json')]) {
    try {
      const parsed = manifestSchema.safeParse(JSON.parse(await readFile(path.join(dir, rel), 'utf8')))
      if (!parsed.success) return null
      const m = parsed.data
      const author = typeof m.author === 'string' ? m.author : m.author && typeof m.author === 'object' && typeof (m.author as { name?: unknown }).name === 'string' ? (m.author as { name: string }).name : null
      return { name: m.name ?? path.basename(dir), version: m.version ?? null, description: m.description ?? '', author, ...(m.skills !== undefined ? { skills: m.skills } : {}) }
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'ENOENT') return null // bozuk manifest → eklenti değil
    }
  }
  return null
}

async function countMatching(dir: string, test: (name: string) => boolean): Promise<number> {
  try {
    return (await readdir(dir)).filter(test).length
  } catch {
    return 0
  }
}

async function countSkills(dir: string): Promise<number> {
  try {
    const entries = await readdir(dir, { withFileTypes: true })
    let n = 0
    for (const e of entries) {
      if (!e.isDirectory()) continue
      try {
        await readFile(path.join(dir, e.name, 'SKILL.md'), 'utf8')
        n++
      } catch {
        // SKILL.md yoksa skill değil
      }
    }
    return n
  } catch {
    return 0
  }
}

async function contributionsOf(dir: string, manifestSkills?: string | string[]): Promise<PluginContributions> {
  const md = (n: string) => n.endsWith('.md')
  const [commands, skills, agents, tools] = await Promise.all([
    countMatching(path.join(dir, 'commands'), md),
    Promise.all(pluginSkillDirs(dir, manifestSkills).map((d) => countSkills(d))).then((ns) => ns.reduce((a, b) => a + b, 0)),
    countMatching(path.join(dir, 'agents'), md),
    countMatching(path.join(dir, 'tools'), (n) => n.endsWith('.json')),
  ])
  return { commands, skills, agents, tools }
}

async function scanBase(base: string, source: 'user' | 'project', disabled: Set<string>, out: Map<string, DiscoveredPlugin>): Promise<void> {
  let entries
  try {
    entries = await readdir(base, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    if (!e.isDirectory()) continue
    const dir = path.join(base, e.name)
    const manifest = await readManifest(dir)
    if (!manifest) continue
    out.set(manifest.name, { name: manifest.name, dirName: path.basename(dir), dir, source, manifest, enabled: !disabled.has(manifest.name), contributions: await contributionsOf(dir, manifest.skills), managed: existsSync(path.join(dir, '.git')) }) // proje sonra: aynı adı ezer
  }
}

/** Tüm eklentileri (etkin+pasif) manifest ve katkı sayılarıyla keşfeder. */
export async function discoverPlugins(root: string | null, trusted: boolean): Promise<DiscoveredPlugin[]> {
  const disabled = await loadDisabled()
  const byName = new Map<string, DiscoveredPlugin>()
  await scanBase(path.join(USER_CONFIG_DIR, 'plugins'), 'user', disabled, byName)
  if (root && trusted) await scanBase(path.join(projectDotDir(root), 'plugins'), 'project', disabled, byName)
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name))
}

/** Etkin eklentilerin kök dizinleri (yükleyiciler alt klasörlerini — tools/commands/skills/agents — ekler). */
export async function enabledPluginDirs(root: string | null, trusted: boolean): Promise<Array<{ name: string; dir: string; source: 'user' | 'project'; skillDirs: string[] }>> {
  return (await discoverPlugins(root, trusted)).filter((p) => p.enabled).map((p) => ({ name: p.name, dir: p.dir, source: p.source, skillDirs: pluginSkillDirs(p.dir, p.manifest.skills) }))
}
