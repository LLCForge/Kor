import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { z } from 'zod'
import { isSensitivePath } from './permissions.ts'
import { scrubEnv } from './secrets.ts'
import { isWithin } from './workspace.ts'

const LSP_TIMEOUT_MS = 60_000
const MAX_SOURCE_BYTES = 2_000_000
const MAX_LOCATIONS = 200

export const CODE_INTEL_ACTIONS = ['definition', 'references', 'hover', 'diagnostics'] as const
export type CodeIntelAction = (typeof CODE_INTEL_ACTIONS)[number]

const LANGUAGE_IDS: Readonly<Record<string, string>> = {
  '.ts': 'typescript', '.mts': 'typescript', '.cts': 'typescript', '.tsx': 'typescriptreact',
  '.js': 'javascript', '.mjs': 'javascript', '.cjs': 'javascript', '.jsx': 'javascriptreact',
}

/**
 * Projede typescript@7 (Go portu) kurulu: klasik JS language service API'si yok, ancak ikili `--lsp --stdio`
 * ile tam bir LSP sunucusu sunuyor. Yalnızca uygulamanın kendi kurulumu kullanılır: incelenen projenin
 * node_modules'ündeki bir ikiliyi çalıştırmak, depodan gelen kodu izinsiz çalıştırmak olurdu.
 */
export function findTypeScriptServer(): string {
  // [g16] Paketlenmiş masaüstü: typescript bir devDependency olduğundan asar'a girmez; dil sunucusu ikilisi ve lib.*.d.ts
  // dosyaları extraResources ile resources/typescript altına kopyalanır (scripts/dist-config.ts).
  const resources = (process as { resourcesPath?: string }).resourcesPath
  const bundled = resources ? path.join(resources, 'typescript', process.platform === 'win32' ? 'tsc.exe' : 'tsc') : null
  if (bundled && existsSync(bundled)) return bundled
  // CJS paketinde (masaüstü) require vardır; ESM'de (CLI, testler) başlatılan betiğin konumundan çözülür.
  // shortcut: ESM tarafı process.argv[1]'e dayanır; uygulama başka biçimde başlatılırsa import.meta.resolve'a geçilir.
  const req = typeof require === 'function' ? require : createRequire(path.resolve(process.argv[1] ?? 'index.js'))
  let packageJson: string
  try {
    packageJson = req.resolve('typescript/package.json')
  } catch {
    throw new Error('TypeScript kurulu değil; code_intel için uygulamanın bağımlılıkları kurulmalı')
  }
  const { version } = z.object({ version: z.string() }).parse(JSON.parse(readFileSync(packageJson, 'utf8')))
  if (Number(version.split('.')[0]) < 7) throw new Error(`code_intel TypeScript 7+ dil sunucusu gerektirir (bulunan ${version})`)
  let platformJson: string
  try {
    platformJson = createRequire(packageJson).resolve(`@typescript/typescript-${process.platform}-${process.arch}/package.json`)
  } catch {
    throw new Error(`Bu platform için TypeScript ikilisi yok (${process.platform}-${process.arch})`)
  }
  const exe = path.join(path.dirname(platformJson), 'lib', process.platform === 'win32' ? 'tsc.exe' : 'tsc')
  if (!existsSync(exe)) throw new Error(`TypeScript ikilisi bulunamadı: ${exe}`)
  return exe
}

const messageSchema = z.object({
  id: z.union([z.number(), z.string()]).nullish(),
  method: z.string().optional(),
  params: z.unknown().optional(),
  result: z.unknown().optional(),
  error: z.object({ message: z.string() }).optional(),
})

/** Content-Length çerçeveli JSON-RPC üzerinden en küçük LSP istemcisi. */
class LspConnection {
  readonly #child: ChildProcess
  #buffer = Buffer.alloc(0)
  #nextId = 1
  #stderr = ''
  readonly #pending = new Map<number, { resolve: (value: unknown) => void; reject: (err: Error) => void }>()

  constructor(exe: string, cwd: string) {
    this.#child = spawn(exe, ['--lsp', '--stdio'], { cwd, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env: scrubEnv(process.env) })
    this.#child.stdout?.on('data', (chunk: Buffer) => this.#onData(chunk))
    this.#child.stderr?.on('data', (chunk: Buffer) => {
      this.#stderr = (this.#stderr + chunk.toString('utf8')).slice(-2000)
    })
    this.#child.on('error', (err) => this.#failAll(err))
    this.#child.on('exit', (code) => this.#failAll(new Error(`Dil sunucusu kapandı (kod ${code ?? '?'}) ${this.#stderr.trim()}`.trim())))
    this.#child.stdin?.on('error', (err) => this.#failAll(err))
  }

  #failAll(err: Error): void {
    for (const p of this.#pending.values()) p.reject(err)
    this.#pending.clear()
  }

  #send(message: Record<string, unknown>): void {
    const json = JSON.stringify({ jsonrpc: '2.0', ...message })
    this.#child.stdin?.write(`Content-Length: ${Buffer.byteLength(json, 'utf8')}\r\n\r\n${json}`)
  }

  #onData(chunk: Buffer): void {
    this.#buffer = Buffer.concat([this.#buffer, chunk])
    for (;;) {
      const headerEnd = this.#buffer.indexOf('\r\n\r\n')
      if (headerEnd === -1) return
      const length = Number(/content-length:\s*(\d+)/i.exec(this.#buffer.subarray(0, headerEnd).toString('ascii'))?.[1])
      if (!Number.isFinite(length)) {
        this.#failAll(new Error('Dil sunucusundan geçersiz ileti başlığı'))
        this.#child.kill()
        return
      }
      const start = headerEnd + 4
      if (this.#buffer.length < start + length) return
      const body = this.#buffer.subarray(start, start + length).toString('utf8')
      this.#buffer = this.#buffer.subarray(start + length)
      let parsed
      try {
        parsed = messageSchema.safeParse(JSON.parse(body))
      } catch {
        continue
      }
      if (parsed.success) this.#dispatch(parsed.data)
    }
  }

  #dispatch(msg: z.infer<typeof messageSchema>): void {
    if (msg.method !== undefined) {
      // Sunucu istekleri yanıtsız kalırsa sunucu bekleyebilir; hepsine boş yanıt verilir.
      if (msg.id !== undefined && msg.id !== null) {
        const items = z.object({ items: z.array(z.unknown()) }).safeParse(msg.params)
        const result = msg.method === 'workspace/configuration' && items.success ? items.data.items.map(() => null) : null
        this.#send({ id: msg.id, result })
      }
      return
    }
    if (typeof msg.id !== 'number') return
    const pending = this.#pending.get(msg.id)
    if (!pending) return
    this.#pending.delete(msg.id)
    if (msg.error) pending.reject(new Error(`Dil sunucusu hatası: ${msg.error.message}`))
    else pending.resolve(msg.result ?? null)
  }

  request(method: string, params: unknown): Promise<unknown> {
    const id = this.#nextId++
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject })
      this.#send({ id, method, params })
    })
  }

  notify(method: string, params: unknown): void {
    this.#send({ method, params })
  }

  async close(): Promise<void> {
    try {
      await Promise.race([this.request('shutdown', null), new Promise((r) => setTimeout(r, 1000).unref())])
      this.notify('exit', null)
    } catch {
      // Sunucu zaten kapanmış olabilir; aşağıda süreç her durumda sonlandırılır.
    } finally {
      this.#child.kill()
    }
  }
}

const positionSchema = z.object({ line: z.number(), character: z.number() })
const rangeSchema = z.object({ start: positionSchema, end: positionSchema })
const locationSchema = z.union([
  z.object({ uri: z.string(), range: rangeSchema }),
  z.object({ targetUri: z.string(), targetSelectionRange: rangeSchema }).transform((l) => ({ uri: l.targetUri, range: l.targetSelectionRange })),
])
const locationsSchema = z.union([locationSchema.transform((l) => [l]), z.array(locationSchema), z.null().transform(() => [])])
const markedSchema = z.union([z.string(), z.object({ value: z.string() })])
const hoverSchema = z
  .object({ contents: z.union([markedSchema, z.array(markedSchema)]) })
  .nullable()
const diagnosticsSchema = z.object({
  items: z
    .array(
      z.object({
        range: rangeSchema,
        severity: z.number().optional(),
        code: z.union([z.number(), z.string()]).optional(),
        message: z.string(),
      }),
    )
    .optional(),
})

const SEVERITY: Readonly<Record<number, string>> = { 1: 'error', 2: 'warning', 3: 'info', 4: 'hint' }

export interface CodeIntelRequest {
  action: CodeIntelAction
  root: string
  /** resolveInWorkspace ile doğrulanmış mutlak yol */
  file: string
  line?: number | undefined
  column?: number | undefined
}

function withTimeout<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  const combined = AbortSignal.any([signal, AbortSignal.timeout(LSP_TIMEOUT_MS)])
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => reject(new Error(signal.aborted ? 'İptal edildi' : `Dil sunucusu ${LSP_TIMEOUT_MS / 1000} sn içinde yanıt vermedi`))
    if (combined.aborted) return onAbort()
    combined.addEventListener('abort', onAbort, { once: true })
    promise.then(resolve, reject).finally(() => combined.removeEventListener('abort', onAbort))
  })
}

export async function runCodeIntel(req: CodeIntelRequest, signal: AbortSignal): Promise<string> {
  const languageId = LANGUAGE_IDS[path.extname(req.file).toLowerCase()]
  if (!languageId) throw new Error('code_intel yalnızca TS/JS dosyalarını destekler (.ts .tsx .js .jsx .mts .cts .mjs .cjs)')
  const text = await readFile(req.file, 'utf8')
  if (Buffer.byteLength(text) > MAX_SOURCE_BYTES) throw new Error('Dosya code_intel için çok büyük (sınır 2 MB)')
  const lines = text.split('\n')
  let position: { line: number; character: number } | null = null
  if (req.action !== 'diagnostics') {
    if (req.line === undefined || req.column === undefined) throw new Error(`${req.action} için line ve column gerekli (1 tabanlı)`)
    if (req.line > lines.length) throw new Error(`Satır ${req.line} yok; dosya ${lines.length} satır`)
    position = { line: req.line - 1, character: req.column - 1 }
  }

  const uri = pathToFileURL(req.file).href
  const rootUri = pathToFileURL(req.root).href
  const conn = new LspConnection(findTypeScriptServer(), req.root)
  try {
    const run = async (): Promise<unknown> => {
      await conn.request('initialize', {
        processId: process.pid,
        rootUri,
        workspaceFolders: [{ uri: rootUri, name: path.basename(req.root) }],
        capabilities: {
          textDocument: { hover: { contentFormat: ['markdown', 'plaintext'] }, definition: {}, references: {}, diagnostic: {} },
          workspace: { configuration: true, workspaceFolders: true },
        },
      })
      conn.notify('initialized', {})
      conn.notify('textDocument/didOpen', { textDocument: { uri, languageId, version: 1, text } })
      const doc = { textDocument: { uri } }
      switch (req.action) {
        case 'definition':
          return conn.request('textDocument/definition', { ...doc, position })
        case 'references':
          return conn.request('textDocument/references', { ...doc, position, context: { includeDeclaration: true } })
        case 'hover':
          return conn.request('textDocument/hover', { ...doc, position })
        case 'diagnostics':
          return conn.request('textDocument/diagnostic', doc)
      }
    }
    const result = await withTimeout(run(), signal)
    return await formatResult(req, result, text)
  } finally {
    await conn.close()
  }
}

async function formatResult(req: CodeIntelRequest, result: unknown, sourceText: string): Promise<string> {
  const rel = (p: string) => (isWithin(req.root, p) ? path.relative(req.root, p).split(path.sep).join('/') : p)
  if (req.action === 'hover') {
    const hover = hoverSchema.parse(result)
    if (!hover) return 'Bu konumda tip bilgisi yok.'
    const parts = Array.isArray(hover.contents) ? hover.contents : [hover.contents]
    return parts.map((p) => (typeof p === 'string' ? p : p.value)).join('\n\n').trim() || 'Bu konumda tip bilgisi yok.'
  }
  if (req.action === 'diagnostics') {
    const items = diagnosticsSchema.parse(result).items ?? []
    if (items.length === 0) return `${rel(req.file)}: tanılama yok.`
    return items
      .map((d) => {
        const code = d.code === undefined ? '' : ` TS${d.code}`
        return `${rel(req.file)}:${d.range.start.line + 1}:${d.range.start.character + 1} - ${SEVERITY[d.severity ?? 1] ?? 'error'}${code}: ${d.message}`
      })
      .join('\n')
  }
  const locations = locationsSchema.parse(result)
  if (locations.length === 0) return req.action === 'definition' ? 'Tanım bulunamadı.' : 'Referans bulunamadı.'
  const cache = new Map<string, string[] | null>([[req.file, sourceText.split('\n')]])
  const out: string[] = []
  for (const loc of locations.slice(0, MAX_LOCATIONS)) {
    let file: string
    try {
      file = fileURLToPath(loc.uri)
    } catch {
      continue
    }
    // Kod satırı yalnızca çalışma dizinindeki, hassas olmayan dosyalardan alıntılanır.
    if (!cache.has(file)) {
      const readable = isWithin(req.root, file) && !isSensitivePath(file)
      cache.set(file, readable ? await readFile(file, 'utf8').then((t) => t.split('\n'), () => null) : null)
    }
    const snippet = cache.get(file)?.[loc.range.start.line]?.trim().slice(0, 200)
    out.push(`${rel(file)}:${loc.range.start.line + 1}:${loc.range.start.character + 1}${snippet ? `: ${snippet}` : ''}`)
  }
  if (locations.length > MAX_LOCATIONS) out.push(`[${locations.length - MAX_LOCATIONS} sonuç daha var]`)
  return out.join('\n')
}
