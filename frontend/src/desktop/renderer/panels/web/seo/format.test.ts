import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { SeoFinding } from '../../../../ipc-seo.ts'
import { counts, filterFindings, foldAgent, location, parseUrls, usd } from './format.ts'

const f = (severity: SeoFinding['severity'], file: string, rule: string, line = 1): SeoFinding => ({ rule, source: 'kor', severity, file, line, message: 'm' })

test('counts and location', () => {
  const list = [f('high', 'https://x.test/', 'seo.title-missing'), f('low', 'https://x.test/a', 'seo.img-alt-missing', 12)]
  assert.deepEqual(counts(list), { critical: 0, high: 1, medium: 0, low: 1, info: 0 })
  assert.equal(location(list[0]), 'https://x.test/')
  assert.equal(location(list[1]), 'https://x.test/a:12')
})

test('filter by severity and text', () => {
  const list = [f('high', 'https://x.test/', 'seo.title-missing'), f('low', 'https://x.test/a', 'seo.img-alt-missing'), f('info', 'u', 'seo.param-url')]
  assert.equal(filterFindings(list, 'medium', '').length, 1)
  assert.equal(filterFindings(list, 'info', 'img').length, 1)
  assert.equal(filterFindings(list, 'info', '').length, 3)
})

test('parseUrls keeps http(s), dedups', () => {
  assert.deepEqual(parseUrls('https://a.test/x\nhttps://a.test/x, ftp://no  http://b.test'), ['https://a.test/x', 'http://b.test'])
})

test('agent events fold into cards', () => {
  let cards = foldAgent([], { state: 'started', agent: 'crawler' })
  cards = foldAgent(cards, { state: 'progress', agent: 'crawler', detail: 'https://x.test/' })
  cards = foldAgent(cards, { state: 'skipped', agent: 'log', reason: 'no log' })
  cards = foldAgent(cards, { state: 'done', agent: 'crawler', findings: 4 })
  assert.deepEqual(cards, [
    { id: 'crawler', state: 'done', findings: 4, detail: '' },
    { id: 'log', state: 'skipped', findings: 0, detail: 'no log' },
  ])
  assert.equal(usd(null), 'bilinmiyor')
  assert.equal(usd(0.01234), '≈ $0.0123')
})
