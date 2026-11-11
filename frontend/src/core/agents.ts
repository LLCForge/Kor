import { readdir, readFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url' // [g29]
import { z } from 'zod'
import { enabledPluginDirs } from './plugins.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { LEGACY_PROJECT_DOT, LEGACY_USER_DIR, PROJECT_DOT, USER_DIR_NAME } from './layout-g28.ts' // [g28]

/** Aynı anda çalışabilecek alt ajan sayısı (ön plan + arka plan). */
export const MAX_CONCURRENT_SUBAGENTS = 4

export interface AgentDefinition {
  name: string
  description: string
  /** 'all' = agent aracı hariç tüm araçlar (alt ajanlar alt ajan başlatamaz) */
  tools: readonly string[] | 'all'
  model: string | null
  prompt: string
  source: 'builtin' | 'embedded' | 'user' | 'project' // [g29] embedded: uygulamayla gelen tanım
  /** [cdx] Eklentiden geliyorsa eklenti adı (Ayarlar > Ajanlar'da rozet) */
  origin?: string
  /** true: alt ajanın değiştiren her işlemi mod ne olursa olsun onay ister (plan modu korunur) */
  manualApproval?: boolean
}

// read_image bilerek yok: görsel desteklemeyen modelde API hatası verir.
const READ_ONLY_TOOLS = ['read_file', 'list_files', 'grep', 'bash_output', 'read_pdf', 'read_notebook', 'code_intel'] as const

export const BUILTIN_AGENTS: readonly AgentDefinition[] = [
  {
    name: 'general-purpose',
    description: 'Çok adımlı, araştırma + değişiklik gerektiren genel görevler. Tüm araçları kullanabilir.',
    tools: 'all',
    model: null,
    prompt:
      'You are a sub-agent launched by the main agent for one delegated task. Complete it end to end with your tools. ' +
      'Read what you need, change what is needed, and prove it works (run the thing) before you report. ' +
      'Your final message is returned to the main agent as the tool result: make it a concise, self-contained report ' +
      '(what you did, what you found, file paths with line numbers, anything left undone). The main agent cannot see your intermediate steps.',
    source: 'builtin',
  },
  {
    name: 'Explore',
    description: 'Hızlı, salt okunur kod tabanı keşfi: dosya bulma, arama, yapıyı anlama. Dosya değiştiremez.',
    tools: READ_ONLY_TOOLS,
    model: null,
    prompt:
      'You are a read-only exploration sub-agent. Search and read the codebase to answer the question. Prefer grep and list_files over reading whole files. ' +
      'You cannot modify anything, but you are not a grep wrapper: read the actual code, follow the call chain, and say what is really there. ' +
      'If you find a bug, a trap or a contradiction on the way, put it in the report even if it was not asked. ' +
      'Final message: findings only, with exact file paths and line numbers, no filler.',
    source: 'builtin',
  },
  {
    name: 'Plan',
    description: 'Salt okunur inceleme yapıp uygulama planı çıkarır: adımlar, etkilenen dosyalar, riskler.',
    tools: READ_ONLY_TOOLS,
    model: null,
    prompt:
      'You are a read-only planning sub-agent. Investigate the relevant code, then return an implementation plan: ordered steps, ' +
      'files to change with reasons, edge cases, risks, and how to verify. Do not modify anything. ' +
      'Be opinionated: one recommended path, not a menu. Where you are unsure, say what you would check first. ' +
      'If a plan step is blocked by a bogus constraint (a stale rule, an "unsupported" label, a sandbox default), say how to break it.',
    source: 'builtin',
  },
  {
    name: 'Review',
    description: 'Kod incelemesi: değişiklikleri okur, hata/güvenlik/performans sorunlarını önem sırasıyla raporlar. Dosya değiştiremez; test çalıştırmak için bash yalnızca kullanıcı onayıyla.',
    tools: [...READ_ONLY_TOOLS, 'bash'],
    model: null,
    prompt:
      'You are a read-only code review sub-agent. Review the code or change described in your task: read the relevant files and their callers, ' +
      'look for correctness bugs, security issues, missing error handling, edge cases, performance problems, and test gaps. ' +
      'You may run the test suite, typecheck, or linters with bash (the user approves each command); stay read-only by default, but when a one-off command ' +
      'would settle a question (a grep, a git log, a curl, a decompiler), say what it would do and run it. ' +
      'Do not modify anything. Final message: findings ordered by severity (critical, high, medium, low), ' +
      'each with file path and line number, why it is a problem, and a concrete fix. If you find nothing significant, say so plainly - do not invent findings.',
    source: 'builtin',
    manualApproval: true,
  },
]

const frontmatterSchema = z.object({
  name: z.string().regex(/^[A-Za-z0-9_-]{1,40}$/, 'name yalnızca harf, rakam, - ve _ içerebilir (en fazla 40)'),
  description: z.string().min(1).max(500),
  tools: z.string().max(500).optional(),
  model: z.string().min(1).max(200).optional(),
})

// Gömülü ajanlar Claude Code araç adlarıyla gelir (Read, Grep, Bash...). Uygulamanın araçları snake_case olduğundan
// eşlenir; yoksa alt ajan araçsız kalırdı. Uygulama kendi adlarını kullanırsa (read_file) olduğu gibi geçer.
const TOOL_ALIASES: Readonly<Record<string, string>> = {
  read: 'read_file',
  write: 'write_file',
  edit: 'edit_file',
  multiedit: 'edit_file',
  grep: 'grep',
  glob: 'list_files',
  ls: 'list_files',
  bash: 'bash',
  bashoutput: 'bash_output',
  webfetch: 'web_fetch',
  websearch: 'web_search',
  todowrite: 'todo_write',
  notebookedit: 'edit_file',
  readnotebook: 'read_notebook',
  readpdf: 'read_pdf',
}
function mapToolName(name: string): string {
  return TOOL_ALIASES[name.toLowerCase().replace(/[_-]/g, '')] ?? name
}

/** Basit YAML frontmatter: --- / anahtar: değer / --- / gövde */
export function parseAgentFile(text: string, source: 'embedded' | 'user' | 'project'): AgentDefinition {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text)
  if (!match) throw new Error('Dosya --- ile başlayan bir frontmatter içermeli')
  const fields: Record<string, string> = {}
  for (const line of (match[1] ?? '').split(/\r?\n/)) {
    const kv = /^([A-Za-z_]+)\s*:\s*(.*)$/.exec(line.trim())
    if (kv?.[1]) fields[kv[1]] = (kv[2] ?? '').replace(/^["']|["']$/g, '')
  }
  const meta = frontmatterSchema.parse(fields)
  const body = (match[2] ?? '').trim()
  if (!body) throw new Error('Ajan istemi (frontmatter altındaki metin) boş olamaz')
  if (body.length > 20_000) throw new Error('Ajan istemi en fazla 20.000 karakter olabilir')
  const tools = meta.tools?.trim()
  return {
    name: meta.name,
    description: meta.description,
    tools: !tools || tools === '*' ? 'all' : [...new Set(tools.split(',').map((t) => mapToolName(t.trim())).filter(Boolean))],
    // Modelden bağımsızlık: gömülü ajanlar ve Claude'a özgü model adları (opus/sonnet/haiku/claude*) yok sayılır — bu adlar
    // Venice'te bulunmaz ve ajanı var olmayan bir modele bağlamak "tüm modellerle uyumlu" olma amacını bozar; kullanıcının
    // seçtiği model kullanılır. Kullanıcı/proje ajanı geçerli bir Venice modeli belirtirse o korunur.
    model: source === 'embedded' || (meta.model ? /^(opus|sonnet|haiku|claude)/i.test(meta.model) : false) ? null : (meta.model ?? null),
    prompt: body,
    source,
  }
}

async function loadDir(dir: string, source: 'embedded' | 'user' | 'project', warn: (msg: string) => void, origin?: string): Promise<AgentDefinition[]> {
  let names: string[]
  try {
    names = await readdir(dir)
  } catch {
    return []
  }
  const defs: AgentDefinition[] = []
  for (const name of names.filter((n) => n.endsWith('.md')).sort()) {
    try {
      const def = parseAgentFile(await readFile(path.join(dir, name), 'utf8'), source)
      defs.push(origin ? { ...def, origin } : def)
    } catch (err) {
      warn(`${path.join(dir, name)}: ${err instanceof Error ? err.message : String(err)}`)
    }
  }
  return defs
}

function uniqueDirs(dirs: readonly string[]): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (const dir of dirs) {
    const key = process.platform === 'win32' ? path.resolve(dir).toLowerCase() : path.resolve(dir)
    if (seen.has(key)) continue
    seen.add(key)
    out.push(dir)
  }
  return out
}

/** Kullanıcı ajanları: önce eski ev klasörü, sonra yeni. Test/taşınabilir override gerçek evi taramaz. */
function userAgentDirs(): string[] {
  const override = process.env.KOR_HOME || process.env.VENICE_AGENT_HOME
  if (override) return [path.join(override, 'agents')]
  return uniqueDirs([
    path.join(os.homedir(), LEGACY_USER_DIR, 'agents'),
    path.join(os.homedir(), USER_DIR_NAME, 'agents'),
    path.join(USER_CONFIG_DIR, 'agents'),
  ])
}

/** Proje ajanları: eski ve yeni nokta klasörü. Biri boşsa diğeri durur; kaynak silinmez. */
function projectAgentDirs(root: string): string[] {
  return uniqueDirs([
    path.join(root, LEGACY_PROJECT_DOT, 'agents'),
    path.join(root, PROJECT_DOT, 'agents'),
  ])
}

/** [g29] Uygulamayla gelen ajan dosyaları. Kaynak çalıştırmada modülün yanında, pakette dist yanında. */
function embeddedAgentDirs(): string[] {
  // Paketlemede import.meta.url build define ile __filename tabanlı URL'ye çevrilir; yine de tanımsız kalırsa
  // (regresyon/beklenmedik durum) gömülü ajanlar atlanır, yerleşik dört ajan ve sohbet çalışmaya devam eder.
  let here: string
  try {
    here = path.dirname(fileURLToPath(import.meta.url))
  } catch {
    return []
  }
  return uniqueDirs([path.join(here, 'builtin-agents'), path.join(here, 'src', 'core', 'builtin-agents')])
}

function isReservedBuiltin(name: string): boolean {
  return BUILTIN_AGENTS.some((b) => b.name === name)
}

/**
 * Yerleşik dört ajan + gömülü tanımlar + kullanıcı ve proje ajan klasörleri + etkin eklentiler.
 * Gömülü tanımlar güven kapısından bağımsız her zaman yüklenir. Aynı isimde diskteki kopya gömülü olanı ezer.
 * Yerleşik adlar ezilemez. Eski dosyalar silinmez.
 * Not: proje tanımları depoyla gelir (AGENTS.md ile aynı güven seviyesi); araç çağrıları yine izin sisteminden geçer.
 */
export async function loadAgentDefinitions(root: string, warn: (msg: string) => void = () => undefined, trusted = false): Promise<AgentDefinition[]> {
  const byName = new Map<string, AgentDefinition>()
  // [g29] Gömülü ajanlar depo talimatı değildir; klasör güvenilmese de yüklenir. Yerleşik dört adı ezemez.
  for (const def of (await Promise.all(embeddedAgentDirs().map((dir) => loadDir(dir, 'embedded', warn)))).flat()) {
    if (isReservedBuiltin(def.name)) {
      warn(`"${def.name}" yerleşik bir ajan adı; gömülü tanım yok sayıldı`)
      continue
    }
    byName.set(def.name, def)
  }
  const custom = [
    ...(await Promise.all(userAgentDirs().map((dir) => loadDir(dir, 'user', warn)))).flat(),
    ...(await Promise.all(projectAgentDirs(root).map((dir) => loadDir(dir, 'project', warn)))).flat(),
  ]
  for (const p of await enabledPluginDirs(root, trusted)) custom.push(...(await loadDir(path.join(p.dir, 'agents'), p.source, warn, p.name))) // eklenti ajanları
  for (const def of custom) {
    if (isReservedBuiltin(def.name)) {
      warn(`"${def.name}" yerleşik bir ajan adı; özel tanım yok sayıldı`)
      continue
    }
    byName.set(def.name, def)
  }
  return [...BUILTIN_AGENTS, ...byName.values()]
}

/** Basit sayaçlı semafor: alt ajan eşzamanlılığını sınırlar. */
export class Semaphore {
  #available: number
  readonly #waiters: Array<() => void> = []

  constructor(size: number) {
    this.#available = size
  }

  async acquire(signal?: AbortSignal): Promise<() => void> {
    // Zaten iptal edilmiş sinyal için 'abort' bir daha tetiklenmez: bekleme kuyruğuna girerse tur askıda kalır.
    if (signal?.aborted) throw signal.reason instanceof Error ? signal.reason : new Error('İptal edildi')
    if (this.#available > 0) {
      this.#available--
    } else {
      await new Promise<void>((resolve, reject) => {
        const onAbort = () => {
          const i = this.#waiters.indexOf(grant)
          if (i >= 0) this.#waiters.splice(i, 1)
          reject(signal?.reason instanceof Error ? signal.reason : new Error('İptal edildi'))
        }
        const grant = () => {
          signal?.removeEventListener('abort', onAbort)
          resolve()
        }
        signal?.addEventListener('abort', onAbort, { once: true })
        this.#waiters.push(grant)
      })
    }
    let released = false
    return () => {
      if (released) return
      released = true
      const next = this.#waiters.shift()
      if (next) next()
      else this.#available++
    }
  }
}
