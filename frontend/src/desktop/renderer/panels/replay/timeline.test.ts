import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { ReplayEvent, ReplayStep } from '../../../ipc-replay.ts'
import { clamp, delayMs, detail, diffAt, diffFiles, seek } from './timeline.ts'

const events: ReplayEvent[] = [
  { kind: 'prompt', t: 0, text: 'fix', model: 'p/m' },
  { kind: 'diff', t: 1, files: [{ path: 'a.rs', before: 'a', after: 'b' }] },
  { kind: 'tool_start', t: 1.5, name: 'Edit', summary: 'a.rs' },
  { kind: 'usage', t: 9.5, input: 10, output: 2, cost_usd: 0.0012, cost_estimated: true, model: 'p/m' },
  { kind: 'diff', t: 10, files: [{ path: 'b.rs', before: 'x', after: null, truncated: true }] },
  { kind: 'end', t: 12, stop: 'Done(Stop)' },
]
const steps: ReplayStep[] = events.map((e, index) => ({ index, t: e.t, kind: e.kind, title: e.kind }))

test('diffAt follows the position both ways', () => {
  assert.equal(diffAt(steps, events, 0), null)
  assert.equal(diffAt(steps, events, 3), events[1])
  assert.equal(diffAt(steps, events, 4), events[4])
  assert.equal(diffAt(steps, events, 99), events[4])
  assert.equal(diffAt([], [], 0), null)
})

test('seek, clamp and playback delay', () => {
  assert.equal(seek(steps, 9.6), 3)
  assert.equal(seek(steps, -1), 0)
  assert.equal(clamp(-3, 6), 0)
  assert.equal(clamp(8, 6), 5)
  assert.equal(clamp(2, 0), 0)
  assert.equal(delayMs(steps, 2, 1), 2000)
  assert.equal(delayMs(steps, 2, 8), 1000)
  assert.equal(delayMs(steps, 5, 1), 0)
})

test('diff files and details are shaped for the UI', () => {
  assert.deepEqual(diffFiles(events[4] ?? null), [{ path: 'b.rs', before: 'x', after: '', truncated: true }])
  assert.deepEqual(diffFiles(null), [])
  assert.deepEqual(detail(events[3]).map((d) => d.text), ['p/m', '10 / 2', '≈$0.0012'])
  assert.deepEqual(detail(events[5]), [{ label: 'Bitiş', text: 'Done(Stop)' }])
})
