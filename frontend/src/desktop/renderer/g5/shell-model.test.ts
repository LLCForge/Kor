// node --test src/desktop/renderer/g5/shell-model.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { type ShellRow, shellStateText, splitShells } from './shell-model.ts'

const row = (shell: string, o: Partial<ShellRow> = {}): ShellRow => ({ shell, path: `/${shell}`, installed: false, present: false, isDefault: false, ...o })

test('present shells come first with the default on top; absent ones are separate', () => {
  const { present, absent } = splitShells([row('bash', { present: true }), row('zsh'), row('fish', { installed: true }), row('powershell', { present: true, isDefault: true })])
  assert.deepEqual(present.map((s) => s.shell), ['powershell', 'bash', 'fish'])
  assert.deepEqual(absent.map((s) => s.shell), ['zsh'])
})

test('state text never reads as if the shell itself were missing', () => {
  assert.equal(shellStateText(row('powershell', { present: true })), 'Entegrasyon kurulu değil')
  assert.equal(shellStateText(row('powershell', { present: true, installed: true })), 'Entegrasyon kurulu')
  assert.equal(shellStateText(row('zsh')), 'Bu bilgisayarda yok')
})
