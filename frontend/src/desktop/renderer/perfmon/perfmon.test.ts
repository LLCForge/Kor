// node --test src/desktop/renderer/perfmon/perfmon.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { percentile, recordEvent, recordIpc, resetStats, setSlowSink, snapshot, type SlowEntry } from './stats.ts'
import { causeOf } from './frames.ts'
import { THREAD_WINDOW, isToggle, stat, syntheticThread, throttled, windowed } from './util.ts'

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
