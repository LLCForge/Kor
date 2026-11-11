// node --test src/desktop/renderer/panels/graph/layout.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { ALPHA_MIN, LINK_DISTANCE, adoptPositions, createSim, fitView, nearest, settle, tick, toScreen, toWorld, zoomAt } from './layout.ts'
import type { SnapshotNode } from '../../../ipc-graph.ts'
import { nodesForFiles, pathSegments, referenceFor, topCoChange, visibleMask, ago } from './model.ts'

const dist = (s: ReturnType<typeof createSim>, a: number, b: number) => Math.hypot(s.x[a]! - s.x[b]!, s.y[a]! - s.y[b]!)

test('başlangıç konumları deterministik ve üst üste binmiyor', () => {
  const a = createSim(50, [], Array.from({ length: 50 }, (_, i) => i % 3))
  const b = createSim(50, [], Array.from({ length: 50 }, (_, i) => i % 3))
  assert.deepEqual([...a.x], [...b.x])
  const pts = new Set([...a.x].map((x, i) => `${x.toFixed(3)},${a.y[i]!.toFixed(3)}`))
  assert.equal(pts.size, 50)
})

test('yerleşim soğur ve durur; bağlı düğümler bağlı olmayanlardan yakın kalır', () => {
  const n = 40
  const edges: Array<[number, number]> = []
  for (let i = 1; i < 20; i++) edges.push([0, i]) // 0 merkezli yıldız
  for (let i = 21; i < 40; i++) edges.push([20, i]) // 20 merkezli ikinci yıldız
  const sim = createSim(n, edges, Array.from({ length: n }, (_, i) => (i < 20 ? 0 : 1)))
  settle(sim, 1000)
  assert.ok(sim.alpha < ALPHA_MIN)
  for (let i = 0; i < n; i++) assert.ok(Number.isFinite(sim.x[i]!) && Number.isFinite(sim.y[i]!))
  // Yay uzunluğu mertebesinde: merkez–yaprak mesafesi, iki merkez arası mesafeden küçük.
  const leaf = dist(sim, 0, 5)
  assert.ok(leaf < LINK_DISTANCE * 3, `yaprak ${leaf}`)
  assert.ok(dist(sim, 0, 20) > leaf, 'iki topluluk ayrık')
})

test('itme üst üste düşen düğümleri ayırır; sabitlenen düğüm yerinde kalır', () => {
  const sim = createSim(3, [], [0, 0, 0])
  sim.x.fill(0)
  sim.y.fill(0)
  sim.pinned[2] = 1
  sim.x[2] = 500
  for (let i = 0; i < 50; i++) tick(sim)
  assert.ok(dist(sim, 0, 1) > 5)
  assert.equal(sim.x[2], 500)
})

test('canlı güncellemede konumlar kimliğe göre korunur', () => {
  const old = createSim(3, [], [0, 0, 0])
  old.x[1] = 777
  old.y[1] = -5
  const next = createSim(2, [], [0, 0])
  assert.equal(adoptPositions(next, ['b', 'new'], old, ['a', 'b', 'c']), 1)
  assert.equal(next.x[0], 777)
  assert.equal(next.y[0], -5)
})

test('3000 düğüm: bir adım bir kare bütçesinde kalır', () => {
  const n = 3000
  const edges: Array<[number, number]> = []
  for (let i = 1; i < n; i++) edges.push([i, Math.floor(i / 3)])
  const sim = createSim(n, edges, Array.from({ length: n }, (_, i) => i % 12))
  for (let i = 0; i < 5; i++) tick(sim) // ısınma
  const t0 = performance.now()
  for (let i = 0; i < 10; i++) tick(sim)
  const per = (performance.now() - t0) / 10
  assert.ok(per < 40, `adım ${per.toFixed(1)} ms`)
})

test('yakınlaştırma imlecin altındaki noktayı sabit tutar; sınırlar korunur', () => {
  const v = { tx: 10, ty: 20, k: 1 }
  const before = toWorld(v, 100, 80)
  const z = zoomAt(v, 2, 100, 80)
  assert.equal(z.k, 2)
  const after = toWorld(z, 100, 80)
  assert.ok(Math.abs(before[0] - after[0]) < 1e-9 && Math.abs(before[1] - after[1]) < 1e-9)
  assert.equal(zoomAt(v, 1e6, 0, 0).k, 8)
  assert.equal(zoomAt(v, 1e-6, 0, 0).k, 0.03)
})

test('sığdırma tüm düğümleri alana koyar; en yakın düğüm bulunur', () => {
  const sim = createSim(4, [], [0, 0, 0, 0])
  const pts = [[-100, -50], [100, 50], [0, 0], [40, -20]] as const
  pts.forEach(([x, y], i) => {
    sim.x[i] = x
    sim.y[i] = y
  })
  const v = fitView(sim, 400, 200)
  for (let i = 0; i < 4; i++) {
    const [sx, sy] = toScreen(v, sim.x[i]!, sim.y[i]!)
    assert.ok(sx >= 0 && sx <= 400 && sy >= 0 && sy <= 200, `${sx},${sy}`)
  }
  assert.equal(nearest(sim, 38, -18, 10), 3)
  assert.equal(nearest(sim, 38, -18, 10, (i) => i !== 3), -1)
})

const node = (id: string, file: string | null, extra: Partial<SnapshotNode> = {}): SnapshotNode => ({
  id,
  kind: 'function',
  name: id,
  file,
  line: 3,
  community: 0,
  degree: 1,
  churn: 0,
  ...extra,
})

test('model: filtre, dosya vurgusu, yol ve başvuru metni', () => {
  const nodes = [node('a', 'src/a.rs'), node('src/a.rs', 'src/a.rs', { kind: 'module' }), node('b', 'src\\b.rs', { community: 2 })]
  assert.deepEqual([...visibleMask(nodes, { kinds: new Set(['function']), community: null })], [1, 0, 1])
  assert.deepEqual([...visibleMask(nodes, { kinds: new Set(['function', 'module']), community: 2 })], [0, 0, 1])
  assert.deepEqual([...nodesForFiles(nodes, ['./src/a.rs'])], [0, 1])
  assert.deepEqual([...nodesForFiles(nodes, ['src/b.rs'])], [2])
  const ref = (id: string) => ({ node: { id, kind: 'function' as const, name: id, file: null, line: null }, edge: null, forward: true })
  assert.deepEqual(pathSegments([ref('a'), ref('missing'), ref('b')], new Map([['a', 0], ['b', 2]])), [])
  assert.deepEqual(pathSegments([ref('a'), ref('b')], new Map([['a', 0], ['b', 2]])), [[0, 2]])
  assert.equal(referenceFor(nodes[0]!), '@src/a.rs — `a` (fonksiyon, satır 3)')
  assert.equal(referenceFor(nodes[1]!), '@src/a.rs')
  assert.equal(referenceFor({ id: 'env:KEY', kind: 'env_var', name: 'KEY', file: null, line: null }), 'Graf düğümü `env:KEY` (ortam değişkeni)')
  const link = (file: string, ratio: number) => ({ node: { id: file, kind: 'module' as const, name: file, file, line: null }, kind: { kind: 'co_changes' as const, ratio }, confidence: { level: 'inferred' as const, score: ratio } })
  assert.deepEqual(topCoChange({ co_changes: [link('x', 0.2), link('y', 0.7)] }), { file: 'y', ratio: 0.7 })
  assert.equal(topCoChange({ co_changes: [] }), null)
  assert.equal(ago(100, 103_000), '3 sn önce')
  assert.equal(ago(null, 0), 'hiç')
})
