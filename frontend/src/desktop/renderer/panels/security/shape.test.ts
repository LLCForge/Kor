// node --test src/desktop/renderer/panels/security/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { AuditEntry, AuditVerify } from '../../../ipc-security.ts'
import { decisionLabel, entrySummary, pageInfo, scopeLabel, validVaultName, verifyText } from './shape.ts'

const base: AuditEntry = { ts: '2026-11-10T10:00:00Z', action: 'tool', prev: '0', hash: '1' }
const ok: AuditVerify = { path: 'p', entries: 3, files: 1, ok: true, line: null, reason: null, file: null, head: 'ok', trimmed: false }

test('entry summaries', () => {
  assert.equal(entrySummary({ ...base, tool: 'Bash', target: 'cargo test' }), 'Bash · cargo test')
  assert.equal(entrySummary({ ...base, tool: 'Read' }), 'Read')
  assert.equal(entrySummary({ ...base, action: 'task', text: 'ilk satır\nikinci' }), 'ilk satır')
  assert.equal(entrySummary({ ...base, action: 'task' }), '')
  assert.equal(entrySummary({ ...base, action: 'model', input: 10, output: 2, cost_usd: '0.001200', cost_estimated: true, masked: 1 }), '10 → 2 token · ≈$0.0012 · 1 sır maskelendi')
  assert.equal(entrySummary({ ...base, action: 'model', input: 10, output: 2 }), '10 → 2 token')
  assert.equal(entrySummary({ ...base, action: 'sandbox_denied', target: '/etc/passwd' }), '/etc/passwd')
  assert.equal(decisionLabel('approved'), 'kullanıcı onayladı')
  assert.equal(decisionLabel(undefined), '')
})

test('page bounds', () => {
  assert.deepEqual(pageInfo(0, 0, 50), { from: 0, to: 0, hasPrev: false, hasNext: false })
  assert.deepEqual(pageInfo(120, 0, 50), { from: 1, to: 50, hasPrev: false, hasNext: true })
  assert.deepEqual(pageInfo(120, 100, 50), { from: 101, to: 120, hasPrev: true, hasNext: false })
})

test('verify text', () => {
  assert.match(verifyText(ok), /3 kaydın/)
  assert.match(verifyText({ ...ok, entries: 0 }), /boş/)
  assert.match(verifyText({ ...ok, entries: 7, files: 3, trimmed: true }), /3 dosya.*arşivler silinmiş/)
  assert.match(verifyText({ ...ok, ok: false, line: 2, reason: 'hash', file: 'audit-000001.jsonl' }), /audit-000001\.jsonl dosyasında 2\. satırda bozuk: .*değiştirilmiş/)
  assert.match(verifyText({ ...ok, ok: false, head: 'truncated' }), /sonundan girdiler silinmiş/)
  assert.match(verifyText({ ...ok, head: 'missing' }), /Baş dosyası henüz yok/)
})

test('vault names and scopes', () => {
  for (const name of ['DATABASE_URL', '_x', 'a1']) assert.ok(validVaultName(name), name)
  for (const bad of ['', '1A', 'A-B', 'A B', 'A'.repeat(129)]) assert.ok(!validVaultName(bad), bad)
  assert.equal(scopeLabel(null), 'tüm komutlar')
  assert.equal(scopeLabel([]), 'hiçbir komut')
  assert.equal(scopeLabel(['psql', 'cargo']), 'psql, cargo')
})
