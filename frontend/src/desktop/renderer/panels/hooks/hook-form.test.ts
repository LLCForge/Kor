import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { HookFile } from '../../../ipc-ek14.ts'
import { emptyForm, formOf, itemOf, summary } from './hook-form.ts'

const kor: HookFile = { path: '/p/.kor/hooks.toml', format: 'kor', scope: 'project', exists: true }
const claude: HookFile = { path: '/p/.claude/settings.json', format: 'claude', scope: 'project', exists: true }

test('a command hook with conditions round-trips', () => {
  const f = { ...emptyForm('post-edit'), command: ' cargo fmt ', files: 'src/**, tests/**', branch: 'main', timeout: '30' }
  const it = itemOf(f, kor)
  assert.ok(typeof it !== 'string')
  assert.deepEqual([it.command, it.files, it.branch, it.timeout, it.mcp], ['cargo fmt', ['src/**', 'tests/**'], 'main', 30, null])
  assert.deepEqual(formOf(it).files, 'src/**, tests/**')
  assert.equal(summary(it), 'cargo fmt · dosya src/**,tests/** · dal main')
})

test('validation per action and file format', () => {
  assert.match(String(itemOf({ ...emptyForm('pre-edit') }, kor)), /Komut/)
  assert.match(String(itemOf({ ...emptyForm('on-error'), action: 'mcp', mcp: 'lint' }, kor)), /sunucu\/araç/)
  assert.match(String(itemOf({ ...emptyForm('on-error'), action: 'mcp', mcp: 'lint/check', args: '[1]' }, kor)), /nesnesi/)
  const ok = itemOf({ ...emptyForm('on-error'), action: 'mcp', mcp: 'lint/check', args: '{"a":1}' }, kor)
  assert.ok(typeof ok !== 'string' && ok.args === '{"a":1}' && ok.command === null)
  assert.match(String(itemOf({ ...emptyForm('Stop'), action: 'notify', notify: 'bitti' }, claude)), /Claude Code/)
  assert.match(String(itemOf({ ...emptyForm('pre-edit'), command: 'x', timeout: '0' }, kor)), /Zaman aşımı/)
  const n = itemOf({ ...emptyForm('on-task-done'), action: 'notify', notify: 'bitti' }, kor)
  assert.ok(typeof n !== 'string')
  assert.equal(formOf(n).action, 'notify')
})
