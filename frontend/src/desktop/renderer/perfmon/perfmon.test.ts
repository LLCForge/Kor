// node --test src/desktop/renderer/perfmon/perfmon.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { percentile, recordEvent, recordIpc, resetStats, setSlowSink, snapshot, type SlowEntry } from './stats.ts'
import { causeOf } from './frames.ts'
import { THREAD_WINDOW, isToggle, stat, syntheticThread, throttled, windowed } from './util.ts'
import { RERENDER_TRACKING, noteRender, rerenderCounts, rerenderTotal, resetRerenders, sameValue } from './rerenders.ts'

test('yüzdelikler yakın sıra ile, kor_runtime::budgets ile aynı', () => {
  const v = Array.from({ length: 100 }, (_, i) => i + 1)
  assert.equal(percentile(v, 0.5), 50)
  assert.equal(percentile(v, 0.95), 95)
  assert.equal(percentile([7], 0.95), 7)
  assert.equal(percentile([], 0.95), 0)
  assert.deepEqual(stat([3, 1, 2]), { p50: 2, p95: 3, n: 3 })
})

test('IPC sayaçları, olay hızı ve yavaş çağrılar', () => {
  resetStats()
  const slow: SlowEntry[] = []
  setSlowSink((e) => slow.push(e))
  recordIpc('git.status', 4)
  recordIpc('g2SearchFiles', 80)
  recordIpc('perfmon.log', 90)
  snapshot(1000)
  for (let i = 0; i < 30; i++) recordEvent()
  const s = snapshot(4000)
  assert.equal(s.calls, 3)
  assert.equal(s.p95, 90)
  assert.deepEqual(s.last, { method: 'perfmon.log', ms: 90 })
  assert.equal(s.eventsPerSec, 10)
  // perfmon.* kendi gecikmesini bildirmez.
  assert.deepEqual(slow, [{ kind: 'ipc', what: 'g2SearchFiles', ms: 80 }])
  setSlowSink(null)
})

test('uzun karenin nedeni en uzun betikten okunur', () => {
  const e = {
    name: 'long-animation-frame',
    entryType: 'long-animation-frame',
    startTime: 0,
    duration: 120,
    toJSON: () => ({}),
    scripts: [
      { duration: 10, invoker: 'IMG.onload' },
      { duration: 90, sourceFunctionName: 'renderGraph', sourceURL: 'app://x/renderer.js', sourceCharPosition: 1234 },
    ],
  } as unknown as PerformanceEntry
  assert.deepEqual(causeOf(e, 'panel:graph'), { what: 'panel:graph: renderGraph', detail: 'renderer.js:1234' })
  const task = { name: 'self', entryType: 'longtask', startTime: 0, duration: 70, toJSON: () => ({}) } as unknown as PerformanceEntry
  assert.deepEqual(causeOf(task, 'chat'), { what: 'chat: task', detail: undefined })
})

test('Ctrl+Shift+P ve Cmd+Shift+P bindirmeyi açar, Ctrl+P açmaz', () => {
  const k = (o: Partial<KeyboardEvent>) => ({ ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, key: 'p', ...o })
  assert.equal(isToggle(k({ ctrlKey: true, shiftKey: true, key: 'P' })), true)
  assert.equal(isToggle(k({ metaKey: true, shiftKey: true })), true)
  assert.equal(isToggle(k({ ctrlKey: true })), false)
  assert.equal(isToggle(k({ ctrlKey: true, shiftKey: true, altKey: true })), false)
})

test('perf.log gönderimi saniyede en çok 4', () => {
  let t = 0
  const got: SlowEntry[] = []
  const send = throttled((e) => got.push(e), 4, () => t)
  for (let i = 0; i < 10; i++) send({ kind: 'frame', what: String(i), ms: 60 })
  assert.equal(got.length, 4)
  t = 1500
  send({ kind: 'frame', what: 'later', ms: 60 })
  assert.equal(got.length, 5)
})

test('sentetik sohbet sırayla kullanıcı ve yanıt', () => {
  const t = syntheticThread(4)
  assert.deepEqual(t.map((i) => i.kind), ['user', 'assistant', 'user', 'assistant'])
  assert.equal(new Set(t.map((i) => i.id)).size, 4)
})

test('uzun sohbet penceresi son bölümleri çizer, sonsuz pencere hepsini', () => {
  const segs = Array.from({ length: 10_000 }, (_, i) => i)
  const w = windowed(segs, THREAD_WINDOW)
  assert.equal(w.hidden, 10_000 - THREAD_WINDOW)
  assert.equal(w.visible.length, THREAD_WINDOW)
  assert.equal(w.visible[0], 10_000 - THREAD_WINDOW)
  const all = windowed(segs, Number.POSITIVE_INFINITY)
  assert.equal(all.hidden, 0)
  assert.equal(all.visible, segs)
})

test('gereksiz çizim sayacı: değerce aynı yeni props sayılır, kendi durumu ve gerçek değişiklik sayılmaz', () => {
  resetRerenders()
  assert.equal(RERENDER_TRACKING, true)
  const a = { id: 1, items: [{ t: 'x' }], on: undefined }
  // İlk çizim ve aynı props nesnesi (bileşenin kendi durumu değişti): gereksiz değil.
  assert.equal(noteRender('Row', null, a), false)
  assert.equal(noteRender('Row', a, a), false)
  // Üst bileşen her çizimde yeni dizi verdi, değer aynı: gereksiz.
  assert.equal(noteRender('Row', a, { id: 1, items: [{ t: 'x' }], on: undefined }), true)
  // Değer değişti: gerekli.
  assert.equal(noteRender('Row', a, { id: 1, items: [{ t: 'y' }], on: undefined }), false)
  // Yeni kapanış farklı sayılır (yanlış alarm yok).
  assert.equal(noteRender('Btn', { f: () => 1 }, { f: () => 1 }), false)
  assert.deepEqual(rerenderCounts(), { Row: 1 })
  assert.equal(rerenderTotal(), 1)
  resetRerenders()
  assert.equal(rerenderTotal(), 0)
})

test('değer eşitliği sınırlı: derinlik, sınıf örnekleri ve anahtar kümeleri', () => {
  assert.equal(sameValue({ a: [1, 2] }, { a: [1, 2] }), true)
  assert.equal(sameValue({ a: 1 }, { b: 1 }), false)
  assert.equal(sameValue([1], { 0: 1 }), false)
  assert.equal(sameValue(new Date(0), new Date(0)), false)
  const deep = (n: number): unknown => (n === 0 ? 1 : { x: deep(n - 1) })
  assert.equal(sameValue(deep(3), deep(3)), true)
  assert.equal(sameValue(deep(10), deep(10)), false)
})
