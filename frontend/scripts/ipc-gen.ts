// backend/crates/kor-core/schema/ipc.json → src/desktop/ipc.gen.ts (spec §16.3, ADR 0061).
// The schema is the single IPC contract: `$defs` become exported types, `methods` the bridge methods
// (`ns.method` → namespace interface, plain name → root), `events` the `onX` subscriptions
// (`kor:<ns>/<x>`, see renderer/kor-bridge.ts eventName). `node scripts/ipc-gen.ts` rewrites the file
// (untouched when equal), `--check` exits 1 when it is stale; build-ui.ts runs it on every build.
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const FRONTEND = resolve(fileURLToPath(import.meta.url), '../..')
export const SCHEMA_PATH = resolve(FRONTEND, '../backend/crates/kor-core/schema/ipc.json')
export const GEN_PATH = resolve(FRONTEND, 'src/desktop/ipc.gen.ts')
/** Name of the generated root interface (every root method, event and namespace). */
export const ROOT_API = 'KorIpcApi'

export interface Schema {
  $ref?: string
  type?: string | string[]
  const?: unknown
  enum?: unknown[]
  anyOf?: Schema[]
  oneOf?: Schema[]
  allOf?: Schema[]
  not?: Schema
  items?: Schema
  prefixItems?: Schema[]
  minItems?: number
  properties?: Record<string, Schema>
  required?: string[]
  additionalProperties?: Schema | boolean
  description?: string
  'x-ts'?: string
  'x-void'?: boolean
}
export interface Param { name: string; optional?: boolean; schema: Schema }
export interface Method { description?: string; params: Param[]; result: Schema; 'x-ts'?: string; 'x-sync'?: boolean }
export interface IpcEvent { description?: string; param?: string; payload?: Schema }
export interface IpcSchema {
  $defs: Record<string, Schema>
  namespaces: Record<string, { title?: string; description?: string }>
  methods: Record<string, Method>
  events: Record<string, IpcEvent>
}

export function readSchema(): IpcSchema {
  return JSON.parse(readFileSync(SCHEMA_PATH, 'utf8')) as IpcSchema
}

const IDENT = /^[A-Za-z_$][\w$]*$/
const key = (k: string) => (IDENT.test(k) ? k : `'${k.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'`)
const lit = (v: unknown) => (typeof v === 'string' ? `'${v.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'` : JSON.stringify(v))
const refName = (r: string) => r.replace('#/$defs/', '')

function doc(text: string | undefined, indent: string): string[] {
  if (!text) return []
  const lines = text.replace(/\*\//g, '*\\/').split('\n')
  if (lines.length === 1) return [`${indent}/** ${lines[0]} */`]
  return [`${indent}/**`, ...lines.map((l) => `${indent} * ${l}`.trimEnd()), `${indent} */`]
}

/** Parenthesise unions, intersections and function types where an array/intersection needs it. */
function wrap(t: string, s: Schema): string {
  const compound = s.anyOf || s.oneOf || (s.enum && s.enum.length > 1) || (Array.isArray(s.type) && s.type.length > 1) || s.allOf || (s['x-ts'] && /=>|\|/.test(s['x-ts']))
  return compound ? `(${t})` : t
}

export function tsType(s: Schema, indent = ''): string {
  if (s['x-ts'] !== undefined) return s['x-ts']
  if (s.$ref) return refName(s.$ref)
  if (s.const !== undefined) return lit(s.const)
  if (s.enum) return s.enum.map(lit).join(' | ')
  if (s.anyOf || s.oneOf) return (s.anyOf ?? s.oneOf)!.map((x) => (x.allOf ? `(${tsType(x, indent)})` : tsType(x, indent))).join(' | ')
  if (s.allOf) return s.allOf.map((x) => wrap(tsType(x, indent), x)).join(' & ')
  if (s.not && Object.keys(s.not).length === 0) return 'never'
  if (Array.isArray(s.type)) return s.type.map((t) => tsType({ ...s, type: t }, indent)).join(' | ')
  switch (s.type) {
    case 'null': return s['x-void'] ? 'void' : 'null'
    case 'string': return 'string'
    case 'number': case 'integer': return 'number'
    case 'boolean': return 'boolean'
    case 'array':
      if (s.prefixItems) {
        const min = s.minItems ?? s.prefixItems.length
        return `[${s.prefixItems.map((x, i) => `${tsType(x, indent)}${i >= min ? '?' : ''}`).join(', ')}]`
      }
      return s.items ? `${wrap(tsType(s.items, indent), s.items)}[]` : 'unknown[]'
    case 'object': return objectType(s, indent)
  }
  return 'unknown'
}

function members(s: Schema, indent: string): string[] {
  const req = new Set(s.required ?? [])
  const out: string[] = []
  for (const [k, v] of Object.entries(s.properties ?? {})) {
    out.push(...doc(v.description, indent), `${indent}${key(k)}${req.has(k) ? '' : '?'}: ${tsType(v, indent)}`)
  }
  if (s.additionalProperties && typeof s.additionalProperties === 'object') out.push(`${indent}[key: string]: ${tsType(s.additionalProperties, indent)}`)
  return out
}

function objectType(s: Schema, indent: string): string {
  if (!s.properties) {
    if (s.additionalProperties && typeof s.additionalProperties === 'object') return `Record<string, ${tsType(s.additionalProperties, indent)}>`
    return 'object'
  }
  const inner = members(s, `${indent}  `)
  if (!inner.length) return '{}'
  if (inner.join('').length < 100 && !inner.some((l) => l.includes('/**') || l.includes('\n'))) return `{ ${inner.map((l) => l.trim()).join('; ')} }`
  return `{\n${inner.join('\n')}\n${indent}}`
}

function defDecl(name: string, s: Schema, defs: Record<string, Schema>): string[] {
  const d = doc(s.description, '')
  const isObj = (x: Schema) => x.type === 'object' && !!x.properties && !x['x-ts']
  if (isObj(s)) return [...d, `export interface ${name} {`, ...members(s, '  '), '}']
  if (s.allOf && s.allOf.length > 1) {
    const last = s.allOf[s.allOf.length - 1]!
    const bases = s.allOf.slice(0, -1)
    const interfaceLike = (b: Schema): boolean => {
      const d = b.$ref ? defs[refName(b.$ref)] : undefined
      return !!d && (isObj(d) || (!!d.allOf && d.allOf.every((x) => x.$ref ? interfaceLike(x) : isObj(x))))
    }
    if (isObj(last) && bases.every(interfaceLike)) {
      return [...d, `export interface ${name} extends ${bases.map((b) => refName(b.$ref!)).join(', ')} {`, ...members(last, '  '), '}']
    }
  }
  return [...d, `export type ${name} = ${tsType(s)}`]
}

function signature(m: Method, name: string, indent: string): string[] {
  const params = m.params.map((p) => `${p.name}${p.optional ? '?' : ''}: ${tsType(p.schema, indent)}`).join(', ')
  const ret = m['x-sync'] ? tsType(m.result, indent) : `Promise<${tsType(m.result, indent)}>`
  // A bare `new(...)` member would be a construct signature, not a method.
  const n = name === 'new' ? `'new'` : key(name)
  return [...doc(m.description, indent), m['x-ts'] ? `${indent}${n}${m['x-ts']}` : `${indent}${n}(${params}): ${ret}`]
}

function listener(e: IpcEvent, indent: string): string {
  if (!e.payload) return '() => void'
  return `(${e.param ?? 'payload'}: ${tsType(e.payload, indent)}) => void`
}

const pascal = (s: string) => s.replace(/(^|[^A-Za-z0-9])([a-z])/g, (_, _p, c: string) => c.toUpperCase()).replace(/[^A-Za-z0-9]/g, '')

/** `kor:a/b` → namespace `a`, member `onB`; `kor:x` → root `onX`. */
export function eventMember(event: string): { ns: string; member: string } {
  const path = event.replace(/^kor:/, '').split('/')
  const last = path.pop()!
  return { ns: path.join('.'), member: `on${last.charAt(0).toUpperCase()}${last.slice(1)}` }
}

export function render(schema: IpcSchema): string {
  const out: string[] = [
    '// GENERATED from backend/crates/kor-core/schema/ipc.json — do not edit (`node scripts/ipc-gen.ts`, spec §16.3, ADR 0061).',
    '/* eslint-disable */',
    '',
  ]
  for (const [name, s] of Object.entries(schema.$defs)) out.push(...defDecl(name, s, schema.$defs), '')

  // namespace tree: '' = root
  const nsNames = new Set<string>([''])
  for (const m of Object.keys(schema.methods)) if (m.includes('.')) nsNames.add(m.slice(0, m.lastIndexOf('.')))
  for (const e of Object.keys(schema.events)) nsNames.add(eventMember(e).ns)
  for (const n of Object.keys(schema.namespaces)) nsNames.add(n)
  for (const n of [...nsNames]) for (let p = n; p.includes('.'); ) { p = p.slice(0, p.lastIndexOf('.')); nsNames.add(p) }
  const title = (ns: string) => (ns === '' ? ROOT_API : schema.namespaces[ns]?.title ?? `${pascal(ns)}Api`)
  const sorted = [...nsNames].sort()
  for (const ns of sorted) {
    const body: string[] = []
    for (const [k, m] of Object.entries(schema.methods)) {
      const at = k.lastIndexOf('.')
      if ((at < 0 ? '' : k.slice(0, at)) === ns) body.push(...signature(m, k.slice(at + 1), '  '))
    }
    for (const [k, e] of Object.entries(schema.events)) {
      const { ns: ens, member } = eventMember(k)
      if (ens === ns) body.push(...doc(e.description, '  '), `  ${member}(listener: ${listener(e, '  ')}): () => void`)
    }
    for (const child of sorted) {
      if (child === ns || !child.startsWith(ns === '' ? '' : `${ns}.`)) continue
      const rest = ns === '' ? child : child.slice(ns.length + 1)
      if (rest.includes('.')) continue
      body.push(...doc(schema.namespaces[child]?.description, '  '), `  ${key(rest)}: ${title(child)}`)
    }
    out.push(...doc(ns === '' ? 'Every bridge method, event and namespace (`window.kor`).' : `\`${ns}.*\` bridge methods.`, ''))
    out.push(`export interface ${title(ns)} {`, ...body, '}', '')
  }

  out.push('/** Arguments and result of every bridge method, by `ns.method` / root name. */', 'export interface IpcMethods {')
  for (const [k, m] of Object.entries(schema.methods)) {
    const args = m.params.map((p) => `${p.name}${p.optional ? '?' : ''}: ${tsType(p.schema, '    ')}`).join(', ')
    out.push(`  ${lit(k)}: { args: [${args}]; result: ${tsType(m.result, '    ')} }`)
  }
  out.push('}', '', '/** Payload of every `kor:*` event. */', 'export interface IpcEvents {')
  for (const [k, e] of Object.entries(schema.events)) out.push(`  ${lit(k)}: ${e.payload ? tsType(e.payload, '  ') : 'void'}`)
  out.push('}', '', `export const IPC_METHODS = ${JSON.stringify(Object.keys(schema.methods))} as const`, '', `export const IPC_EVENTS = ${JSON.stringify(Object.keys(schema.events))} as const`, '')
  return out.join('\n')
}

export function writeGenerated(): boolean {
  const next = render(readSchema())
  let current = ''
  try { current = readFileSync(GEN_PATH, 'utf8') } catch { /* first run */ }
  if (current === next) return false
  writeFileSync(GEN_PATH, next)
  return true
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--check')) {
    let current = ''
    try { current = readFileSync(GEN_PATH, 'utf8') } catch { /* missing */ }
    if (current !== render(readSchema())) {
      console.error('src/desktop/ipc.gen.ts is stale: run `node scripts/ipc-gen.ts`')
      process.exit(1)
    }
  } else if (writeGenerated()) console.log('ipc.gen.ts written')
}
