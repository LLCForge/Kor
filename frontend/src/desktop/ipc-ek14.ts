// K14 (ADR 0063) IPC sözleşmeleri: konuşma dalları (`fork.*`), canlı diyagramlar (`diagram.*`), kişisel
// bilgi tabanı (`know.*`), tahmin (`estimate.*`), kural uyumu (`rules.*`), özel dokümanlar (`docs.*`),
// hook editörü (`hooks.*`), MCP geliştirici modu (`mcpdev.*`). Rust: frontend/src-tauri/src/ns/{forks,
// diagrams,know,estimate,rules,docs,hooks,mcpdev}.rs.
import type { ReviewFile } from './ipc-review.ts'
import type { RuleFinding } from './ipc.gen.ts'

// ---------------------------------------------------------------- fork (§27.3)
export interface ForkRecord {
  id: string
  parent: string
  at: number
  created: number
  model?: string
  root?: string
  worktree?: string
  branch?: string
  base?: string
  merged?: number
  /** Dallanma anındaki haline getirilen dosyalar (proje göreli). */
  rewound?: string[]
  /** Getirilemeyen dosyalar (şimdiki halleriyle kaldı). */
  unrestored?: string[]
  /** Ebeveyn dallanmadan sonra kabuk komutu çalıştırdı. */
  shellAfter?: boolean
}
export interface ForkNode {
  id: string
  title: string
  at: number | null
  model: string | null
  merged: number | null
  worktree: string | null
  missing: boolean
  children: ForkNode[]
}
export interface ForkSide { id: string; after: number; conclusion: string; dir: string | null; changed: string[] }
export interface ForkFileDiff { path: string; diff: string; added: number; removed: number }
export interface ForkComparison { common: number; a: ForkSide; b: ForkSide; files: ForkFileDiff[] }
export interface MergePlan { fork: string; parent: string; summary: string; target: string | null; files: ReviewFile[]; skipped: string[] }
export interface ForkMessage { n: number; role: string; text: string }
export interface ForkApi {
  messages(id: string): Promise<ForkMessage[]>
  create(id: string, at: number, model: string | null, noWorktree: boolean): Promise<ForkRecord>
  tree(id?: string | null): Promise<ForkNode[]>
  compare(a: string, b: string): Promise<ForkComparison>
  mergePlan(id: string): Promise<MergePlan>
  mergeApply(id: string, files: { path: string; content: string | null }[], note: boolean): Promise<{ written: string[]; note: boolean }>
  remove(id: string, worktree: boolean): Promise<null>
}

// ---------------------------------------------------------------- diagram (§27.14)
export type DiagramStatus = 'up_to_date' | 'stale' | 'edited' | 'invalid'
export interface DiagramChange {
  file: string
  line: number
  kind: 'architecture' | 'dependency' | 'sequence' | 'er' | 'deploy' | null
  status: DiagramStatus
  error: string | null
  counts: { nodes_added: number; nodes_removed: number; edges_added: number; edges_removed: number }
  text: string
  ascii: string[]
}
export interface DiagramApi {
  list(): Promise<DiagramChange[]>
  sync(force: boolean): Promise<{ written: string[]; changes: DiagramChange[]; summary: string[] }>
  add(file: string, kind: string, format: string, params: Record<string, string>): Promise<{ written: string[]; changes: DiagramChange[]; summary: string[] }>
}

// ---------------------------------------------------------------- know (§27.15)
export interface KnowRecord { id: number; ts: number; project: string; kind: string; task: string; summary: string; files: string[]; errors: string[] }
export type StaleReason =
  | { kind: 'project_missing'; project: string }
  | { kind: 'file_missing'; file: string }
  | { kind: 'file_changed'; file: string; modified: number }
  | { kind: 'symbol_gone'; symbol: string }
export interface KnowStale { id: number; task: string; reasons: StaleReason[] }
export interface KnowHit { hit: { record: KnowRecord; score: number }; stale: KnowStale | null }
export interface KnowApi {
  search(query: string, here: boolean): Promise<KnowHit[]>
  digest(week?: string | null): Promise<{ week: string; markdown: string; path: string }>
  digests(): Promise<{ week: string; path: string }[]>
  stale(here: boolean): Promise<KnowStale[]>
}

// ---------------------------------------------------------------- estimate (§27.16)
export interface TaskEstimate {
  secs: number
  usd: number | null
  files: number
  turns: number
  method: 'history' | 'prior'
  samples: number
  model: string
  local: boolean
}
export interface EstimateApi {
  task(text: string, sessionId: string | null, model: string | null): Promise<{ estimate: TaskEstimate; line: string; method: string; warning: string | null } | null>
}

// ---------------------------------------------------------------- rules (§28.3)
export interface RulesApi {
  check(staged: boolean, judge: boolean): Promise<{ findings: RuleFinding[]; files: number; judged: boolean; error: string | null }>
  list(): Promise<{ source: string; line: number; rule: string; checkable: boolean }[]>
  report(days: number): Promise<{ source: string; rule: string; model: string; count: number }[]>
}

// ---------------------------------------------------------------- docs (§28.7)
export interface DocEntry { id: string; source: string; title: string; sha256: string; added: number; updated: number; chars: number; records: number[]; browser: boolean }
export interface DocsApi {
  list(): Promise<DocEntry[]>
  add(source: string, browser: boolean): Promise<DocEntry>
  pick(): Promise<DocEntry | null>
  search(query: string): Promise<{ title: string; text: string; source: string | null }[]>
  refresh(id?: string | null): Promise<{ id: string; changed: boolean; error: string | null }[]>
  remove(id: string): Promise<null>
}

// ---------------------------------------------------------------- hooks (§28.12)
export type HookFormat = 'kor' | 'claude'
export interface HookFile { path: string; format: HookFormat; scope: string; exists: boolean }
export interface HookItem {
  id: string
  file: string
  format: HookFormat | null
  scope: string
  event: string
  matcher: string | null
  command: string | null
  timeout: number | null
  files: string[]
  branch: string | null
  model: string | null
  mcp: string | null
  args: string | null
  notify: string | null
  disabled: boolean
}
export interface HookSample { tool?: string | null; file?: string | null; model?: string | null; branch?: string | null; prompt?: string | null }
export interface HookDryRun { matches: boolean; skipped: string[]; korEvents: string[]; executed: boolean; blocked: string | null; context: string[]; notices: string[]; ms: number }
export interface HookLogEntry { at: number; event: string; command: string; source: string; result: string; ms: number; message: string | null }
export interface HooksApi {
  events(): Promise<{ kor: string[]; claude: string[] }>
  files(): Promise<HookFile[]>
  list(): Promise<{ items: HookItem[]; warnings: string[] }>
  save(fileIndex: number, item: HookItem, replace: { event: string; id: string } | null): Promise<null>
  remove(fileIndex: number, event: string, id: string): Promise<null>
  test(fileIndex: number | null, item: HookItem, sample: HookSample, execute: boolean): Promise<HookDryRun>
  log(limit: number): Promise<HookLogEntry[]>
}

// ---------------------------------------------------------------- mcpdev (§28.15)
export interface McpInventory { server: Record<string, unknown>; tools: McpToolInfo[]; resources: { uri: string; name?: string }[]; prompts: { name: string; description?: string }[] }
export interface McpToolInfo { name: string; description?: string; inputSchema?: { properties?: Record<string, unknown> } }
export interface McpTraffic { ms: number; dir: 'out' | 'in' | 'stderr' | 'note'; text: string }
export interface McpSchemaDiff { added: string[]; removed: string[]; changed: [string, string][] }
export interface McpDevApi {
  start(path: string, command: string | null): Promise<{ id: number; inventory: McpInventory }>
  list(): Promise<{ id: number; path: string; running: boolean }[]>
  inspect(id: number): Promise<McpInventory>
  reload(id: number): Promise<{ inventory: McpInventory; diff: McpSchemaDiff }>
  call(id: number, tool: string, argsJson: string): Promise<unknown>
  read(id: number, uri: string): Promise<unknown>
  prompt(id: number, name: string, argsJson: string): Promise<unknown>
  log(id: number): Promise<McpTraffic[]>
  stop(id: number): Promise<boolean>
  onTraffic(listener: (e: { id: number; traffic: McpTraffic }) => void): () => void
  onReloaded(listener: (e: { id: number; inventory?: McpInventory; diff?: McpSchemaDiff; error?: string }) => void): () => void
}
