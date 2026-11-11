// [g13] Katmanlı sistem istemi: global (~/.kor/system-prompt.md, prompt.ts) < proje (.kor/system-prompt.md) < oturum.
// Bu bölümler her model çağrısından önce sistem mesajında, sınırları belli tek bir blok olarak (system-blocks.ts) tutulur.
// Böylece sistem istemini yeniden kuran her yol (hafıza, güven, proje haritası yenilemesi) katmanları kaybetmez.
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import type { Session } from './agent.ts'
import { loadUserSettingsFile, USER_CONFIG_DIR } from './settings.ts'
import { stripSystemBlock, upsertSystemBlock } from './system-blocks.ts'
import { isFolderTrusted, recordGeneratedFile } from './trust.ts'
import { versionsSection } from './versions-g13.ts'
import { confineToWorkspace } from './workspace.ts'
import { adoptProject } from './layout-g28.ts' // [g28]

/** Depoyla gelen proje katmanı; trust.ts PROJECT_TRUST_FILES içinde (AGENTS.md gibi güven sorusunu tetikler). */
export const PROJECT_PROMPT_FILE = path.join('.kor', 'system-prompt.md')
export const SESSION_PROMPTS_DIR = path.join(USER_CONFIG_DIR, 'session-prompts')
export const MAX_LAYER_CHARS = 16_000

const idSchema = z.string().uuid()

export interface G13SessionState {
  /** undefined: diskten henüz okunmadı; null: oturum katmanı yok */
  sessionPrompt?: string | null
  /** Bloğun hesaplandığı temel istem ve güven durumu: biri değişince blok yeniden hesaplanır */
  base?: string
  trusted?: boolean
  block?: string
}

async function readText(abs: string): Promise<string | null> {
  try {
    return (await readFile(abs, 'utf8')).trim() || null
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

function assertLayer(text: string): void {
  if (text.length > MAX_LAYER_CHARS) throw new Error(`Katman en fazla ${MAX_LAYER_CHARS.toLocaleString('tr')} karakter olabilir`)
}

export async function loadProjectPrompt(root: string): Promise<string | null> {
  adoptProject(root)
  return readText(confineToWorkspace(root, PROJECT_PROMPT_FILE))
}

/** Boş metin dosyayı siler. Uygulamanın yazdığı içerik kaydedilir; aynı kaldıkça güven sorusunu tetiklemez. */
export async function saveProjectPrompt(root: string, text: string): Promise<void> {
  adoptProject(root)
  assertLayer(text)
  const abs = confineToWorkspace(root, PROJECT_PROMPT_FILE)
  if (!text.trim()) {
    await rm(abs, { force: true })
    return
  }
  await mkdir(path.dirname(abs), { recursive: true })
  await writeFile(abs, text, 'utf8')
  await recordGeneratedFile(abs, text)
}

/** id UUID olarak doğrulanır; dosya yolu klasör dışına kaçamaz (CWE-22). */
function sessionPromptFile(id: string, dir: string): string {
  return path.join(dir, `${idSchema.parse(id)}.md`)
}

export async function loadSessionPrompt(id: string, dir: string = SESSION_PROMPTS_DIR): Promise<string | null> {
  return readText(sessionPromptFile(id, dir))
}

export async function saveSessionPrompt(id: string, text: string, dir: string = SESSION_PROMPTS_DIR): Promise<void> {
  assertLayer(text)
  const file = sessionPromptFile(id, dir)
  if (!text.trim()) {
    await rm(file, { force: true })
    return
  }
  await mkdir(dir, { recursive: true })
  await writeFile(file, text, { encoding: 'utf8', mode: 0o600 })
}

/** Kullanıcının onayladığı doküman kaynakları; yalnızca kullanıcı ayar dosyasından (proje dosyası kaynak ekleyemez). */
export async function loadDocSources(): Promise<string[]> {
  return (await loadUserSettingsFile()).docSources ?? []
}

const clip = (text: string) => (text.length > MAX_LAYER_CHARS ? `${text.slice(0, MAX_LAYER_CHARS)}\n[truncated]` : text)

/** Katman yüklenemezse o katman atlanır (kapalı başarısızlık); ayrıntı konsola. */
async function safely<T>(label: string, fn: () => Promise<T>, fallback: T): Promise<T> {
  try {
    return await fn()
  } catch (err) {
    console.warn(`${label} okunamadı:`, err instanceof Error ? err.message : err)
    return fallback
  }
}

export interface BlockInput {
  root: string
  /** Güvenilmeyen klasörde depoyla gelen katman ve sürüm bilgisi isteme girmez */
  trusted: boolean
  sessionPrompt: string | null
}

export async function buildG13Block(input: BlockInput): Promise<string> {
  const sections: string[] = []
  if (input.trusted) {
    const project = await safely('Proje sistem istemi', () => loadProjectPrompt(input.root), null)
    if (project) {
      sections.push(`# PROJECT SYSTEM PROMPT (${PROJECT_PROMPT_FILE.split(path.sep).join('/')})\nProject-level instructions. They refine the global instructions above, and override them where they conflict; the user's newest message still beats every layer.\n${clip(project)}`)
    }
    const versions = await safely('Sürüm bilgisi', () => versionsSection(input.root), '')
    if (versions) sections.push(versions)
  }
  const sources = await safely('Doküman kaynakları', loadDocSources, [])
  if (sources.length > 0) {
    sections.push(
      `# DOC SOURCES\nUser-approved documentation sources. When library behavior or API details for the versions above matter, read the current docs with the docs_fetch tool (docs_fetch accepts only these prefixes; web_fetch reaches anything else):\n${sources.map((s) => `- ${s}`).join('\n')}`,
    )
  }
  if (input.sessionPrompt) {
    sections.push(`# SESSION INSTRUCTIONS\nThe user set these instructions for this chat only; they take precedence over every layer above, including the project layer.\n${clip(input.sessionPrompt)}`)
  }
  return sections.join('\n\n')
}

export function stripG13Block(system: string): string {
  return stripSystemBlock(system, 'g13')
}

/** Birleşik istem: temel istem (global + ortam + AGENTS.md + hafıza ...) ve g13 katmanları; blok yerinde güncellenir. */
export function withG13Block(system: string, block: string): string {
  return upsertSystemBlock(system, 'g13', block)
}

/** Arayüzden katman değişince: bir sonraki model çağrısında blok yeniden hesaplanır. */
export function invalidateG13(session: Session, sessionPrompt?: string | null): void {
  const state = (session.g13 ??= {})
  state.block = undefined
  if (sessionPrompt !== undefined) state.sessionPrompt = sessionPrompt
}

/** runTurn'deki [g13] kancası: sistem mesajındaki g13 bloğunu güncel tutar. */
export async function g13BeforeModelCall(session: Session, root: string): Promise<void> {
  // Alt ajanlar (agents: []) üst oturumun sistem istemini blok dahil devralır; kendi oturum katmanları yoktur.
  if (session.agents.length === 0) return
  const first = session.messages[0]
  if (first?.role !== 'system' || typeof first.content !== 'string') return
  const state = (session.g13 ??= {})
  if (state.sessionPrompt === undefined) {
    state.sessionPrompt = idSchema.safeParse(session.id).success ? await safely('Oturum sistem istemi', () => loadSessionPrompt(session.id), null) : null
  }
  const base = stripG13Block(first.content)
  // Güven her çağrıda okunur (küçük bir JSON): yalnızca .kor/system-prompt.md içeren klasörde güven kararı temel istemi değiştirmeyebilir.
  const trusted = await safely('Klasör güveni', () => isFolderTrusted(root), false)
  if (state.block === undefined || state.base !== base || state.trusted !== trusted) {
    state.block = await buildG13Block({ root, trusted, sessionPrompt: state.sessionPrompt })
    state.base = base
    state.trusted = trusted
  }
  const next = withG13Block(first.content, state.block)
  if (next !== first.content) session.messages[0] = { role: 'system', content: next }
}
