// node --test src/desktop/renderer/panels/triggers/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { TriggerInput } from '../../../ipc-triggers.ts'
import { duration, formFrom, formProblem, forSave, usd } from './shape.ts'

const f = (over: Partial<TriggerInput>): TriggerInput => ({ ...formFrom(null), name: 'ci', task: 'testleri çalıştır', ...over })

test('form problems', () => {
  assert.equal(formProblem(f({})), null)
  assert.equal(formProblem(f({ name: ' ' })), 'Ad gerekli')
  assert.equal(formProblem(f({ name: 'a/b' })), 'Ad / ya da \\ içeremez')
  assert.equal(formProblem(f({ event: 'cron', schedule: '0 3 * * *' })), null)
  assert.match(formProblem(f({ event: 'cron', schedule: '0 3 * *' })) ?? '', /beş alan/)
  assert.match(formProblem(f({ event: 'webhook', path: '/hooks/x' })) ?? '', /HMAC/)
  assert.equal(formProblem(f({ event: 'webhook', path: '/hooks/x', secret: 'DEPLOY_HOOK' })), null)
  assert.match(formProblem(f({ event: 'log_match', source: 'app.log', pattern: '(' })) ?? '', /düzenli ifade/)
  assert.match(formProblem(f({ budgetUsd: -1 })) ?? '', /Bütçe/)
})

test('save drops fields of other events', () => {
  const s = forSave(f({ event: 'cron', schedule: ' 0 3 * * * ', branch: 'main', path: '/x' }))
  assert.equal(s.schedule, '0 3 * * *')
  assert.equal(s.branch, undefined)
  assert.equal(s.path, undefined)
})

test('money and time', () => {
  assert.equal(usd(0.123, true), '≈$0.12')
  assert.equal(usd(0, false), '$0.00')
  assert.equal(duration(null, null, 0), '—')
  assert.equal(duration(0, 65_000, 0), '1 dk 5 sn')
  assert.equal(duration(1000, null, 4000), '3 sn')
})
