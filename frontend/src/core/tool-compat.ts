// [uyum] Her modelde her araç: Claude Code (ve türevleri) adlarıyla gelen araç çağrıları bizim araçlara çevrilir.
// Skiller/ajanlar çoğunlukla Claude Code için yazılmıştır ("Read the file with Read, then Edit…"); model bu adları
// birebir çağırınca "bilinmeyen araç" hatasıyla iş duruyordu. Burada ad + argüman adları çevrilir, büyük/küçük harf
// ve _/- farkları tolere edilir. Saf modül (test edilebilir).

export interface CompatCall { name: string; args: Record<string, unknown> }

type Translator = (args: Record<string, unknown>) => CompatCall

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined)
const num = (v: unknown): number | undefined => (typeof v === 'number' ? v : undefined)

/** Yalnızca verilen anahtarları (adları çevirerek) taşır; bilinmeyenler düşer (şema hatası vermesin). */
function pick(args: Record<string, unknown>, map: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [from, to] of Object.entries(map)) if (args[from] !== undefined) out[to] = args[from]
  return out
}

const TRANSLATORS: Record<string, Translator> = {
  read: (a) => ({ name: 'read_file', args: pick(a, { file_path: 'path', path: 'path', offset: 'offset', limit: 'limit' }) }),
  write: (a) => ({ name: 'write_file', args: pick(a, { file_path: 'path', path: 'path', content: 'content' }) }),
  edit: (a) => ({ name: 'edit_file', args: pick(a, { file_path: 'path', path: 'path', old_string: 'old_string', new_string: 'new_string', replace_all: 'replace_all' }) }),
  multiedit: (a) => ({ name: 'multi_edit', args: { path: str(a.file_path) ?? str(a.path), edits: Array.isArray(a.edits) ? a.edits : [] } }),
  notebookedit: (a) => ({ name: 'edit_notebook', args: pick(a, { notebook_path: 'path', path: 'path', cell_id: 'cell_id', new_source: 'source', source: 'source', cell_type: 'cell_type', edit_mode: 'mode' }) }),
  notebookread: (a) => ({ name: 'read_notebook', args: pick(a, { notebook_path: 'path', path: 'path' }) }),
  bash: (a) => ({ name: 'bash', args: { command: str(a.command) ?? str(a.cmd) ?? '', ...(a.run_in_background !== undefined ? { run_in_background: a.run_in_background } : {}), ...(num(a.timeout) !== undefined ? { timeout_s: Math.ceil(num(a.timeout)! / 1000) } : {}) } }),
  bashoutput: (a) => ({ name: 'bash_output', args: { shell_id: str(a.bash_id) ?? str(a.shell_id) ?? '' } }),
  killshell: (a) => ({ name: 'kill_shell', args: { shell_id: str(a.shell_id) ?? str(a.bash_id) ?? '' } }),
  killbash: (a) => ({ name: 'kill_shell', args: { shell_id: str(a.shell_id) ?? str(a.bash_id) ?? '' } }),
  grep: (a) => ({ name: 'grep', args: { ...pick(a, { pattern: 'pattern', path: 'path', glob: 'glob', head_limit: 'max_results' }), ...(a['-i'] === true || a.ignore_case === true ? { ignore_case: true } : {}) } }),
  glob: (a) => ({ name: 'list_files', args: { pattern: str(a.path) && str(a.pattern) ? `${str(a.path)!.replace(/\/+$/, '')}/${str(a.pattern)}` : (str(a.pattern) ?? '**/*') } }),
  ls: (a) => ({ name: 'list_files', args: { pattern: str(a.path) ? `${str(a.path)!.replace(/\/+$/, '')}/*` : '*' } }),
  task: (a) => ({ name: 'agent', args: { description: str(a.description) ?? 'alt görev', prompt: str(a.prompt) ?? '', subagent_type: str(a.subagent_type) ?? 'general-purpose', ...(a.run_in_background !== undefined ? { run_in_background: a.run_in_background } : {}) } }),
  agent: (a) => ({ name: 'agent', args: { description: str(a.description) ?? 'alt görev', prompt: str(a.prompt) ?? '', subagent_type: str(a.subagent_type) ?? 'general-purpose', ...(a.run_in_background !== undefined ? { run_in_background: a.run_in_background } : {}) } }),
  todowrite: (a) => ({ name: 'todo_write', args: { items: Array.isArray(a.todos) ? (a.todos as Array<Record<string, unknown>>).map((t) => ({ content: str(t.content) ?? str(t.activeForm) ?? '', status: str(t.status) === 'completed' ? 'completed' : str(t.status) === 'in_progress' ? 'in_progress' : 'pending' })) : a.items } }),
  webfetch: (a) => ({ name: 'web_fetch', args: pick(a, { url: 'url', prompt: 'prompt' }) }),
  websearch: (a) => ({ name: 'web_search', args: pick(a, { query: 'query', limit: 'limit' }) }),
  skill: (a) => ({ name: 'use_skill', args: { name: (str(a.skill) ?? str(a.name) ?? '').replace(/^\//, ''), ...(str(a.file) ? { file: str(a.file) } : {}) } }),
  askuserquestion: (a) => ({ name: 'ask_user', args: a }),
  exitplanmode: (a) => ({ name: 'exit_plan_mode', args: a }),
  enterplanmode: () => ({ name: 'exit_plan_mode', args: {} }),
  toolsearch: (a) => ({ name: 'tool_search', args: { query: str(a.query) ?? '' } }),
  readpdf: (a) => ({ name: 'read_pdf', args: pick(a, { file_path: 'path', path: 'path' }) }),
  computeruse: (a) => ({ name: 'computer_screenshot', args: a }),
}

const norm = (name: string) => name.toLowerCase().replace(/[_\-\s]/g, '')

/**
 * Bilinmeyen bir araç adını mevcut araçlara çevirir. Sıra: (1) büyük/küçük harf ve _/- farkı (Read_File → read_file),
 * (2) Claude Code adı + argüman çevirisi (Read{file_path} → read_file{path}), (3) mcp__ öneki eksik MCP adı.
 * Çevrilemezse null.
 */
export function resolveCompatCall(name: string, args: Record<string, unknown>, known: ReadonlySet<string>): CompatCall | null {
  if (known.has(name)) return { name, args }
  const n = norm(name)
  // Önce Claude Code çevirisi (argüman adları da değişir: Bash{timeout} → bash{timeout_s}); sonra yalnızca ad farkı.
  const tr = TRANSLATORS[n]
  if (tr) {
    const out = tr(args)
    if (known.has(out.name)) return out
  }
  for (const k of known) if (norm(k) === n) return { name: k, args }
  // mcp__sunucu__arac yerine "sunucu__arac" ya da yalnız "arac" denmişse
  const tail = (k: string) => k.replace(/^mcp__[^_]+__/, '')
  const mcp = [...known].filter((k) => k.startsWith('mcp__') && (norm(tail(k)) === n || norm(k.slice(5)) === n))
  if (mcp.length === 1) return { name: mcp[0]!, args }
  return null
}

/** Skill/ajan metinlerine eklenen kısa harita: Claude Code adlarıyla yazılmış talimatlar bu uygulamada nasıl uygulanır. */
export const TOOL_NAME_MAP_NOTE =
  '(Tool names in this app: Read→read_file(path), Write→write_file, Edit→edit_file(old_string,new_string), MultiEdit→multi_edit, ' +
  'Bash→bash(command), Grep→grep, Glob/LS→list_files(pattern), Task→agent(prompt,subagent_type), TodoWrite→todo_write, WebFetch→web_fetch, ' +
  'WebSearch→web_search, Skill or a /slash-command→use_skill(name), $ARGUMENTS = the user\'s request. Calling the Claude names also works; they are translated.)'

/** Sağlayıcı araç adı kuralları (OpenAI/Gemini/Groq): [A-Za-z0-9_-], en çok 64 karakter. MCP adları bazen nokta/boşluk içerir. */
export function sanitizeToolName(name: string): string {
  const cleaned = name.replace(/[^A-Za-z0-9_-]/g, '_')
  if (cleaned.length <= 64) return cleaned
  let h = 0
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0
  return `${cleaned.slice(0, 57)}_${h.toString(36).slice(0, 6)}`
}
