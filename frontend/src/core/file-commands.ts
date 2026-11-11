// Dosyadan slash komutları (Claude/Cursor uyumlu): bir komut, argümanlarla genişleyip istem olan bir metin şablonudur.
// Dizinler — proje (yalnızca klasör güveniliyse): <root>/.claude/commands, <root>/.cursor/commands, <root>/.kor/commands.
// Kullanıcı: ~/.claude/commands ve <USER_CONFIG_DIR>/commands. Dosya adı komut adıdır (uzantısız).
// Biçim: isteğe bağlı YAML frontmatter (description, argument-hint), gövdede $ARGUMENTS (tüm argümanlar) ve $1..$9.
import { readdir, readFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { enabledPluginDirs } from './plugins.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { projectDotDir } from './layout-g28.ts' // [g28]

export { expandCommand } from './command-expand.ts'

const MAX_BODY = 20_000
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,48}$/

export interface FileCommand {
  name: string
  description: string
  argumentHint: string | null
  body: string
  source: 'user' | 'project' | 'plugin'
}

/** İsteğe bağlı frontmatter + gövde. description yoksa ilk satırdan türetilir. */
export function parseCommandFile(text: string, name: string, source: FileCommand['source']): FileCommand {
  const fm = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text)
  let description = ''
  let argumentHint: string | null = null
  let body = text.trim()
  if (fm) {
    for (const line of (fm[1] ?? '').split(/\r?\n/)) {
      const kv = /^([A-Za-z-]+)\s*:\s*(.*)$/.exec(line.trim())
      if (!kv?.[1]) continue
      const val = (kv[2] ?? '').replace(/^["']|["']$/g, '').trim()
      if (kv[1].toLowerCase() === 'description') description = val.slice(0, 500)
      else if (kv[1].toLowerCase() === 'argument-hint') argumentHint = val.slice(0, 200) || null
    }
    body = (fm[2] ?? '').trim()
  }
  if (!body) throw new Error('Komut gövdesi boş olamaz')
  if (body.length > MAX_BODY) throw new Error(`Komut gövdesi en fazla ${MAX_BODY} karakter olabilir`)
  if (!description) description = (body.split('\n')[0] ?? name).slice(0, 80)
  return { name, description, argumentHint, body, source }
}

async function loadDir(dir: string, source: FileCommand['source'], byName: Map<string, FileCommand>, warn: (msg: string) => void): Promise<void> {
  let names: string[]
  try {
    names = await readdir(dir)
  } catch {
    return
  }
  for (const file of names.filter((n) => n.endsWith('.md')).sort()) {
    const name = file.slice(0, -3)
    if (!NAME_RE.test(name)) {
      warn(`${path.join(dir, file)}: komut adı harf/rakamla başlamalı (harf, rakam, _ , -)`)
      continue
    }
    try {
      byName.set(name, parseCommandFile(await readFile(path.join(dir, file), 'utf8'), name, source)) // sonra gelen (proje) kazanır
    } catch (err) {
      warn(`${path.join(dir, file)}: ${err instanceof Error ? err.message : String(err)}`)
    }
  }
}

/** homeDir enjekte edilebilir (test); varsayılan gerçek ev dizini (~/.claude/commands okunur). */
export async function loadFileCommands(
  root: string | null,
  trusted: boolean,
  warn: (msg: string) => void = () => undefined,
  homeDir: string = os.homedir(),
): Promise<FileCommand[]> {
  const byName = new Map<string, FileCommand>()
  // Kullanıcı önce yüklenir; güvenilen proje komutları aynı adı ezer (Claude davranışı).
  await loadDir(path.join(homeDir, '.claude', 'commands'), 'user', byName, warn)
  await loadDir(path.join(USER_CONFIG_DIR, 'commands'), 'user', byName, warn)
  if (root && trusted) {
    for (const base of [path.join(root, '.claude', 'commands'), path.join(root, '.cursor', 'commands'), path.join(projectDotDir(root), 'commands')]) {
      await loadDir(base, 'project', byName, warn)
    }
  }
  for (const p of await enabledPluginDirs(root, trusted)) await loadDir(path.join(p.dir, 'commands'), 'plugin', byName, warn) // eklenti komutları: 'plugin' olarak işaretlenir (palet rozeti)
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name))
}
