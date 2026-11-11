import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { SubTask, TaskState } from '../../../ipc-tasks.ts'
import { isActive, modelText, parseModels, percent, totalUsd, upsert, usd } from './shape.ts'

function sub(over: Partial<SubTask>): SubTask {
  return {
    index: 0, role: 'backend', goal: 'g', files: [], status: 'pending', branch: 'b', worktree: 'w', model: null,
    models_used: [], base: null, commit: null, changed: [], outside: [], cost_usd: 0, estimated: false, test: null,
    review: null, error: null, session: null, ...over,
  }
}

function task(over: Partial<TaskState>): TaskState {
  return {
    version: 1, id: '1', goal: 'g', summary: '', status: 'running', created_ms: 0, updated_ms: 0, base: 'x',
    branch: 'kor/task/1/merged', subtasks: [], conflicts: [], test: null, orchestrator_usd: 0, reviewer_usd: 0,
    estimated: false, forked_from: null, error: null,
    settings: { agents: 5, max_parallel: 4, models: {}, budget_usd: 0, budget_per_agent_usd: 0, test_command: '', max_turns: 40 },
    ...over,
  }
}

test('percent counts merged and dropped parts', () => {
  const t = task({ subtasks: [sub({ status: 'merged' }), sub({ status: 'dropped' }), sub({ status: 'conflict' }), sub({})] })
  assert.equal(percent(t), 50)
  assert.equal(percent(task({ status: 'done' })), 100)
  assert.equal(percent(task({})), 0)
})

test('costs add up and estimates are marked', () => {
  const t = task({ orchestrator_usd: 0.1, reviewer_usd: 0.05, subtasks: [sub({ cost_usd: 0.25 })] })
  assert.ok(Math.abs(totalUsd(t) - 0.4) < 1e-9)
  assert.equal(usd(0.4, true), '≈$0.40')
  assert.equal(usd(0.001, false), '<$0.01')
})

test('model text prefers the pinned model', () => {
  assert.equal(modelText(sub({ model: 'a/b', models_used: ['c/d'] })), 'a/b')
  assert.equal(modelText(sub({ models_used: ['c/d', 'e/f'] })), 'c/d, e/f')
  assert.equal(modelText(sub({})), 'Yönlendirici seçer')
})

test('upsert replaces by id and sorts by update time', () => {
  const a = task({ id: '1', updated_ms: 1 })
  const b = task({ id: '2', updated_ms: 2 })
  const a2 = task({ id: '1', updated_ms: 3, status: 'done' })
  const l = upsert(upsert([a], b), a2)
  assert.deepEqual(l.map((x) => x.id), ['1', '2'])
  assert.equal(l[0]!.status, 'done')
  assert.ok(isActive('merging') && !isActive('conflict'))
})

test('role models parse and report bad parts', () => {
  const r = parseModels('backend=openai/gpt-4.1, docs = local/qwen3\nbroken, x=nomodel')
  assert.deepEqual(r.models, { backend: 'openai/gpt-4.1', docs: 'local/qwen3' })
  assert.deepEqual(r.bad, ['broken', 'x=nomodel'])
})
