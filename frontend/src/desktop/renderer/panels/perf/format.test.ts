// node --test src/desktop/renderer/panels/perf/format.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { PerfCompareRow } from '../../../ipc-perf.ts'
import { EMPTY_FORM, changeText, changeTone, location, metricLabel, openable, svgDataUrl, toOptions, weight } from './format.ts'

const row = (over: Partial<PerfCompareRow>): PerfCompareRow => ({ metric: 'wall_ms', subject: null, before: 100, after: 80, changePct: -20, ...over })

test('weights by unit', () => {
  assert.equal(weight(5, 'samples'), '5 örnek')
  assert.equal(weight(1536, 'bytes'), '1.5 KiB')
  assert.equal(weight(3 << 20, 'bytes'), '3.0 MiB')
  assert.equal(weight(45_000, 'microseconds'), '45.0 ms')
  assert.equal(weight(2_500_000, 'nanoseconds'), '2.5 ms')
})

test('locations and which files open in the preview', () => {
  assert.equal(location({ file: 'src/a.rs', line: 3 }), 'src/a.rs:3')
  assert.equal(location({ file: null, line: 3 }), '')
  assert.ok(openable('src/a.rs'))
  for (const bad of [null, '/usr/lib/x.so', 'C:\\x\\y.js', 'file:///C:/a.js', 'node:internal/x', '../x.rs']) assert.ok(!openable(bad), String(bad))
})

test('svg goes in as an image url', () => {
  const u = svgDataUrl('<svg><text>a & b</text></svg>')
  assert.ok(u.startsWith('data:image/svg+xml;charset=utf-8,'))
  assert.ok(!u.includes('<') && !u.includes('&'))
})

test('compare rows: direction and labels', () => {
  assert.equal(changeTone(row({})), 'better')
  assert.equal(changeTone(row({ metric: 'rps', changePct: -20 })), 'worse')
  assert.equal(changeTone(row({ metric: 'rps', changePct: 10 })), 'better')
  assert.equal(changeTone(row({ changePct: null, after: null })), 'same')
  assert.equal(changeText(row({ changePct: 12.5 })), '+12.50%')
  assert.equal(metricLabel(row({ metric: 'top_share', subject: 'fib' })), 'En sıcak fonksiyonun payı (%) · fib')
})

test('form to run options', () => {
  assert.deepEqual(toOptions({ ...EMPTY_FORM, target: ' cargo test ', requests: '50' }), { target: 'cargo test' })
  assert.deepEqual(
    toOptions({ ...EMPTY_FORM, target: 'http://127.0.0.1:3000/x', profiler: 'py-spy', pid: '42', cmd: ' ', duration: '0', concurrency: 'x' }, 'b1'),
    { target: 'http://127.0.0.1:3000/x', profiler: 'py-spy', requests: 200, pid: 42, baseline: 'b1' },
  )
})
