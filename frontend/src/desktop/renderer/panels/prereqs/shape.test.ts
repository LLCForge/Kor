// node --test src/desktop/renderer/panels/prereqs/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { PrereqRow } from '../../../ipc-prereq.ts'
import { actionText, appendLine, reasonText, resultText, statusText } from './shape.ts'

const row = (over: Partial<PrereqRow>): PrereqRow => ({
  id: 'docker',
  installed: false,
  permission: false,
  command: 'winget install --id Docker.DockerDesktop',
  manager: 'winget',
  elevation: 'none',
  action: 'install',
  reason: null,
  ...over,
})

test('status, action and reason per row', () => {
  assert.equal(statusText(row({})), 'Eksik')
  assert.equal(statusText(row({ installed: true })), 'Kurulu')
  assert.equal(statusText(row({ installed: null, permission: true })), 'İlk gerektiğinde sistem sorar')
  assert.equal(actionText(row({})), 'Kur')
  assert.equal(actionText(row({ installed: true })), null)
  assert.equal(actionText(row({ action: 'open-settings', installed: null })), 'Sistem Ayarlarını aç')
  assert.equal(actionText(row({ command: null, reason: 'no-manager' })), null)
  assert.match(reasonText(row({ command: null, reason: 'no-manager' })) ?? '', /çalışmaya devam eder/)
  assert.equal(reasonText(row({})), null)
})

test('results never say "install it first"', () => {
  assert.equal(resultText('docker', { ok: true, code: 0, row: row({ installed: true }) }), 'Docker kuruldu.')
  assert.match(resultText('wsl2', { ok: false, code: null, row: row({ id: 'wsl2' }) }), /kurulmadı\. Kor çalışmaya devam eder/)
  assert.match(resultText('podman', { ok: false, code: 1603, row: row({ id: 'podman' }) }), /çıkış kodu 1603/)
  assert.match(resultText('microphone', { ok: true, code: 0, row: row({ id: 'microphone', action: 'open-settings' }) }), /Sistem Ayarları açıldı/)
})

test('progress keeps the last lines per prerequisite', () => {
  let log: Record<string, string[]> = {}
  for (let i = 0; i < 8; i++) log = appendLine(log, 'docker', `l${i}`)
  log = appendLine(log, 'podman', 'p')
  assert.deepEqual(log.docker, ['l2', 'l3', 'l4', 'l5', 'l6', 'l7'])
  assert.deepEqual(log.podman, ['p'])
})
