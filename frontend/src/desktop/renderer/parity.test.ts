// node --test src/desktop/renderer/parity.test.ts
// Renderer tarafı parite (spec §2 kural 9, §14 madde 5, §23.5; ADR 0031): parity.toml'daki her masaüstü köprü
// yöntemine arayüzden ulaşılır — bir renderer çağıranı ya da paletin "Tüm komutlar" sayfası (panel / genel
// çalıştırıcı) üzerinden. Rust tarafında gerçek eşleşme kolu `frontend/src-tauri/src/ns/mod.rs` testinde.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import {
  DESKTOP_SRC, GEN_PATH, bridgeCalls, korCmdAttrs, readParity, renderGenerated, sourceFiles, wrapperAliases,
} from '../../../scripts/parity-commands.ts'
import { CONFIRM_METHODS, formatResult, panelIdsFor, parseArgs } from './parity-palette.ts'

const PANELS_DIR = join(DESKTOP_SRC, 'renderer/panels')

function scanRenderer() {
  const srcs = sourceFiles(DESKTOP_SRC).map((p) => readFileSync(p, 'utf8'))
  const aliases = new Map<string, string>()
  for (const s of srcs) for (const [a, ns] of wrapperAliases(s)) aliases.set(a, ns)
  const reached = new Set<string>()
  for (const s of srcs) {
    for (const c of bridgeCalls(s, aliases)) reached.add(c)
    for (const c of korCmdAttrs(s)) reached.add(c)
  }
  return reached
}

/** registry.ts'e eklenmiş panellerin kimlikleri (`from './<dir>/index.tsx'` → `id: '<id>'`). */
function registeredPanelIds(): Set<string> {
  const ids = new Set<string>()
  const registry = readFileSync(join(PANELS_DIR, 'registry.ts'), 'utf8')
  for (const m of registry.matchAll(/from\s+['"]\.\/([\w-]+)\/index\.tsx?['"]/g)) {
    const index = [join(PANELS_DIR, m[1]!, 'index.tsx'), join(PANELS_DIR, m[1]!, 'index.ts')].find(existsSync)
    const id = index ? /\bid:\s*['"]([\w-]+)['"]/.exec(readFileSync(index, 'utf8'))?.[1] : undefined
    if (id) ids.add(id)
  }
  return ids
}

test('parity-commands.gen.ts parity.toml ile eşzamanlı (`node scripts/parity-commands.ts`)', () => {
  // A Windows checkout may have CRLF line endings; the content is what must match.
  assert.equal(readFileSync(GEN_PATH, 'utf8').replace(/\r\n/g, '\n'), renderGenerated(readParity()))
})

test('onay listesi yalnızca gerçek parity köprü yöntemlerini içerir', () => {
  const methods = new Set(readParity().map((f) => f.desktop))
  assert.deepEqual([...CONFIRM_METHODS].filter((m) => !methods.has(m)), [])
})

test('her masaüstü köprü yöntemine arayüzden ulaşılır', (t) => {
  const reached = scanRenderer()
  const panels = registeredPanelIds()
  const methods = [...new Set(readParity().map((f) => f.desktop).filter((d) => d !== '-'))]
  const rootUnreached: string[] = []
  const panelUnreached: string[] = []
  const runnerOnly: string[] = []
  for (const m of methods) {
    if (reached.has(m)) continue
    const [ns, name] = m.split('.')
    // Kök yöntemler (send, openSession…) palete girmez: bir çağıranları olmalı.
    if (!name) { rootUnreached.push(m); continue }
    const panel = panelIdsFor(ns!).find((id) => panels.has(id))
    // Palet bu yöntem için paneli açar; panel (ya da başka bir yer) çağırmıyorsa yönteme ulaşılamaz.
    if (panel) panelUnreached.push(`${m} (panel: ${panel})`)
    else runnerOnly.push(m)
  }
  t.diagnostic(`yalnızca genel çalıştırıcıyla ulaşılan (${runnerOnly.length}): ${runnerOnly.join(' ')}`)
  assert.deepEqual(rootUnreached, [], 'çağıranı olmayan kök köprü yöntemleri')
  assert.deepEqual(panelUnreached, [], 'palet paneli açıyor ama hiçbir bileşen bu yöntemi çağırmıyor')
})

test('tarayıcı: zincir, sarmalayıcı ve içe aktarma yolu', () => {
  const aliases = wrapperAliases('export const costApi = (api as unknown as { cost: CostApi }).cost')
  assert.deepEqual([...aliases], [['costApi', 'cost']])
  const twoStep = wrapperAliases('const typed = api as unknown as { memory: M; projmem: P }\nexport const memoryApi = typed.memory\nexport const projmemApi = typed.projmem')
  assert.deepEqual([...twoStep], [['memoryApi', 'memory'], ['projmemApi', 'projmem']])
  const src = "import { api } from '../api.ts'\nvoid api.core\n  .setPrivacy(x)\napi.send(m)\ncostApi.why(1)\nwindow.kor.graph.status()"
  assert.deepEqual([...bridgeCalls(src, aliases)].sort(), ['core.setPrivacy', 'cost.why', 'graph.status', 'send'])
  assert.deepEqual([...korCmdAttrs(`<a data-kor-cmd="a.b c.d" /><b data-kor-cmd={'e.f'} />`)], ['a.b', 'c.d', 'e.f'])
})

test('genel çalıştırıcı: bağımsız değişken ve sonuç biçimi', () => {
  assert.deepEqual(parseArgs(''), { ok: true, args: [] })
  assert.deepEqual(parseArgs('[1, "a"]'), { ok: true, args: [1, 'a'] })
  assert.deepEqual(parseArgs('{"a":1}'), { ok: true, args: [{ a: 1 }] })
  assert.equal(parseArgs('[1,').ok, false)
  assert.equal(parseArgs(`"${'x'.repeat(70_000)}"`).ok, false)
  assert.equal(formatResult(undefined), 'Tamam (sonuç yok)')
  assert.equal(formatResult('metin'), 'metin')
  assert.equal(formatResult({ a: 1 }), '{\n  "a": 1\n}')
  assert.equal(formatResult(new Uint8Array(3)), '3 bayt ikili veri')
  assert.deepEqual(panelIdsFor('bench'), ['bench', 'cost'])
})
