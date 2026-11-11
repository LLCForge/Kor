// node --test src/desktop/renderer/panels/deploy/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { DeployPlan, DeployStep } from '../../../ipc-deploy.ts'
import { defaultApproved, reduceDeploy, riskCounts, rowsOf, shortSha } from './shape.ts'

const step = (id: number, over: Partial<DeployStep> = {}): DeployStep => ({
  id,
  kind: 'build',
  program: 'docker',
  args: [],
  env: [],
  display: `docker step ${id}`,
  approve: true,
  destructive: false,
  service: null,
  ...over,
})

const plan = (steps: DeployStep[]): DeployPlan => ({
  env: 'prod',
  target: 'compose',
  head: 'abcdef1234567890',
  since: null,
  dirty: false,
  changed_files: null,
  services: ['web'],
  all_services: ['web'],
  steps,
  rollback_steps: [],
  rollback: 'ask',
  health_timeout: 120,
  color: null,
  notes: [],
})

test('events become step rows; extra steps are appended', () => {
  let rows = rowsOf(plan([step(1), step(2, { kind: 'up' })]))
  rows = reduceDeploy(rows, { type: 'start', step: step(1) })
  rows = reduceDeploy(rows, { type: 'output', id: 1, line: 'built' })
  rows = reduceDeploy(rows, { type: 'done', id: 1, ok: true, detail: '' })
  rows = reduceDeploy(rows, { type: 'skipped', id: 2 })
  assert.deepEqual(rows.map((r) => r.state), ['ok', 'skipped'])
  assert.deepEqual(rows[0]?.output, ['built'])
  const inspect = step(1, { kind: 'snapshot', display: 'docker inspect x' })
  rows = reduceDeploy(rows, { type: 'start', step: inspect })
  rows = reduceDeploy(rows, { type: 'done', id: 1, ok: false, detail: 'exit code 1' })
  assert.equal(rows.length, 3)
  assert.equal(rows[2]?.state, 'failed')
  assert.equal(rows[0]?.state, 'ok')
})

test('destructive and read steps are not pre-approved', () => {
  const p = plan([step(1), step(2, { kind: 'health', approve: false }), step(3, { kind: 'stop', destructive: true })])
  assert.deepEqual(defaultApproved(p), [1])
  assert.equal(shortSha('abcdef1234567890'), 'abcdef123456')
  assert.equal(shortSha(null), '–')
  assert.deepEqual(riskCounts([{ level: 'high', kind: 'migration', title: '', file: null, detail: '' }]), { high: 1, medium: 0, info: 0 })
})
