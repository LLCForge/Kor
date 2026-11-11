// node --test scripts/ipc-gen.test.ts
// §16.3 (ADR 0061): kor-core/schema/ipc.json is the one IPC contract. These checks fail when the generated TS
// drifts from it, or when the renderer names a command/event the schema (and so the Rust side) does not have.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { relative } from 'node:path'
import { GEN_PATH, eventMember, readSchema, render } from './ipc-gen.ts'
import { DESKTOP_SRC, korCmdAttrs, readParity, bridgeFeatures, sourceFiles, wrapperAliases } from './parity-commands.ts'

const schema = readSchema()
const methods = new Set(Object.keys(schema.methods))
const events = new Set(Object.keys(schema.events))

test('ipc.gen.ts is generated from the current schema', () => {
  assert.equal(readFileSync(GEN_PATH, 'utf8'), render(schema), 'run `node scripts/ipc-gen.ts`')
})

test('every $ref resolves and every method has params and a result', () => {
  const bad: string[] = []
  const walk = (n: unknown, at: string) => {
    if (n === null || typeof n !== 'object') return
    for (const [k, v] of Object.entries(n)) {
      if (k === '$ref' && typeof v === 'string' && !(v.replace('#/$defs/', '') in schema.$defs)) bad.push(`${at}: ${v}`)
      walk(v, `${at}/${k}`)
    }
  }
  walk(schema, '')
  for (const [name, m] of Object.entries(schema.methods)) {
    if (!Array.isArray(m.params) || m.result === undefined) bad.push(`${name}: params/result`)
  }
  assert.deepEqual(bad, [])
})

test('every data-kor-cmd / data-kor-event in the renderer names a schema method / event', () => {
  const problems: string[] = []
  for (const p of sourceFiles(DESKTOP_SRC).filter((f) => f.endsWith('.tsx'))) {
    const src = readFileSync(p, 'utf8')
    for (const c of korCmdAttrs(src)) if (!methods.has(c)) problems.push(`${relative(DESKTOP_SRC, p)}: data-kor-cmd="${c}"`)
    for (const m of src.matchAll(/data-kor-event=(?:"([^"]*)"|\{\s*['"`]([^'"`]*)['"`]\s*\})/g)) {
      for (const e of (m[1] ?? m[2] ?? '').split(/\s+/)) {
        if (!e) continue
        // `kor:ns/…` (namespace wildcard) is allowed when the schema has an event in that namespace.
        const ok = e.endsWith('/…') || e.endsWith('/*') ? [...events].some((x) => x.startsWith(e.slice(0, -1))) : events.has(e)
        if (!ok) problems.push(`${relative(DESKTOP_SRC, p)}: data-kor-event="${e}"`)
      }
    }
  }
  assert.deepEqual(problems, [])
})

test('every bridge call in the renderer is a schema method or event', () => {
  const files = sourceFiles(DESKTOP_SRC).filter((f) => /\.tsx?$/.test(f) && !f.endsWith('.gen.ts') && !f.endsWith('.test.ts') && !f.endsWith('.check.ts'))
  // Typed-cast aliases (`const costApi = (api as unknown as {…}).cost`), visible where defined or imported by name.
  const exported = new Map<string, string>()
  for (const p of files) for (const [a, ns] of wrapperAliases(readFileSync(p, 'utf8'))) exported.set(a, ns)
  const subs = new Set([...events].map((e) => { const { ns, member } = eventMember(e); return ns ? `${ns}.${member}` : member }))
  const problems: string[] = []
  for (const p of files) {
    const src = readFileSync(p, 'utf8')
    const local = wrapperAliases(src)
    for (const [a, ns] of exported) if (new RegExp(String.raw`import\s[^;]*\b${a}\b[^;]*from`).test(src)) local.set(a, ns)
    const calls = new Set<string>()
    // `api` counts only where it is the bridge (`import { api } from '…/api.ts'`), not a local of that name.
    const bridgeApi = /import\s*\{[^}]*\bapi\b[^}]*\}\s*from\s*'[^']*\/api(?:\.ts)?'/.test(src)
    const callRe = bridgeApi ? /(?<![\w/.$-])(?:api|window\.kor)\s*\.\s*(\w+)(?:\s*\.\s*(\w+))?/g : /(?<![\w/.$-])window\.kor\s*\.\s*(\w+)(?:\s*\.\s*(\w+))?/g
    for (const m of src.matchAll(callRe)) calls.add(m[2] ? `${m[1]}.${m[2]}` : m[1]!)
    for (const [alias, ns] of local) for (const m of src.matchAll(new RegExp(String.raw`(?<![\w/.$-])${alias}\s*\.\s*(\w+)`, 'g'))) calls.add(`${ns}.${m[1]}`)
    for (const call of calls) {
      // `api.ns` alone (passing the namespace object on) is a namespace, not a call.
      if (/(^|\.)on[A-Z]/.test(call) ? subs.has(call) : methods.has(call) || [...methods].some((m) => m.startsWith(`${call}.`))) continue
      problems.push(`${relative(DESKTOP_SRC, p)}: ${call}`)
    }
  }
  assert.deepEqual(problems, [])
})

test('every desktop surface in parity.toml is a schema method', () => {
  const missing = bridgeFeatures(readParity()).map((f) => f.desktop).filter((d) => !methods.has(d))
  assert.deepEqual(missing, [])
})

test('generated method and event lists equal the schema', () => {
  const gen = readFileSync(GEN_PATH, 'utf8')
  const list = (name: string) => JSON.parse(new RegExp(`export const ${name} = (\\[.*\\]) as const`).exec(gen)![1]!) as string[]
  assert.deepEqual(list('IPC_METHODS').sort(), [...methods].sort())
  assert.deepEqual(list('IPC_EVENTS').sort(), [...events].sort())
})
