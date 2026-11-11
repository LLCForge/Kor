import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { HealthAlert } from '../../../ipc-system.ts'
import { bytes, controlling, diskUsed, mb, pushCard, validApp, validLayout } from './shape.ts'

const alert = (id: string): HealthAlert => ({ id, kind: 'disk', level: 'warn', title: id, body: '', clean: true })

test('boyutlar 1024 tabanlı', () => {
  assert.equal(bytes(0), '0 B')
  assert.equal(bytes(1536), '1.5 kB')
  assert.equal(mb(18 * 1024), '18.0 GB')
  assert.equal(bytes(200 * 1024 * 1024), '200 MB')
})

test('disk doluluğu', () => {
  assert.equal(diskUsed({ mount: 'C:', name: '', fs: '', total_mb: 100, free_mb: 4, removable: false }), 96)
  assert.equal(diskUsed({ mount: 'C:', name: '', fs: '', total_mb: 0, free_mb: 0, removable: false }), 0)
})

test('kart yığını tekrar etmez ve sınırlıdır', () => {
  let cards: HealthAlert[] = []
  for (const id of ['a', 'b', 'a', 'c', 'd']) cards = pushCard(cards, alert(id))
  assert.deepEqual(cards.map((c) => c.id), ['d', 'c', 'a'])
})

test('gösterge 5 sn sonra söner, duraklatınca hemen', () => {
  const a = { active: true, tool: 'left_click', app: 'mspaint', paused: null, at: 1000 }
  assert.equal(controlling(a, 3000), true)
  assert.equal(controlling(a, 7000), false)
  assert.equal(controlling({ ...a, paused: 'mouse' }, 1000), false)
  assert.equal(controlling(null, 0), false)
})

test('ad doğrulama', () => {
  assert.ok(validApp('visual studio code') && validApp('chrome'))
  assert.ok(!validApp('a.b') && !validApp(''))
  assert.ok(validLayout('kodlama-2') && !validLayout('.gizli') && !validLayout('a/b'))
})
