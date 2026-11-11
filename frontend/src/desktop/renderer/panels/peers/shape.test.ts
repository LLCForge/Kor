import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { PeerRow } from '../../../ipc.gen.ts'
import { fromForm, grantText, loadText, qrCells, READ_ONLY, sortPeers, toForm } from './shape.ts'

const row = (name: string, score: number | null): PeerRow => ({
  name,
  fingerprint: 'ab12-cd34-ef56-7890',
  addrs: [],
  ssh: null,
  grant: READ_ONLY,
  load: score == null ? null : { name, cpus: 4, busy: score, tasks: 0, version: '0.8.0' },
  score,
  error: score == null ? 'timed out' : null,
})

test('en boş makine önce, ulaşılamayan sonda', () => {
  const s = sortPeers([row('c', null), row('b', 0.9), row('a', 0.1)])
  assert.deepEqual(s.map((r) => r.name), ['a', 'b', 'c'])
  assert.equal(loadText(row('a', 0.25).load), '0 görev · 4 çekirdek · %25 dolu')
  assert.equal(loadText(null), 'ulaşılamıyor')
})

test('yetki formu gidiş-dönüş, varsayılan salt-okunur', () => {
  assert.equal(grantText(READ_ONLY), 'salt-okunur')
  const f = toForm(READ_ONLY)
  const g = fromForm({ ...f, dirs: '/srv/app\n\n/srv/app\n', commands: 'cargo test\nnpm run', memory: true })
  assert.deepEqual(g.dirs, ['/srv/app'])
  assert.deepEqual(g.commands, ['cargo test', 'npm run'])
  assert.equal(grantText(g), '1 dizin (okuma) · komut: cargo test, npm run · hafıza')
})

test('QR hücreleri', () => {
  assert.deepEqual(qrCells(2, [true, false, false, true]), [[0, 0], [1, 1]])
  assert.deepEqual(qrCells(1, []), [])
})
