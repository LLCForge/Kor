// node --test src/desktop/renderer/osint/osextra-model.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { OsExtraStatus } from '../../ipc-osextra.ts'
import { LABELS, reasonText, rowAction } from './osextra-model.ts'

const item = (o: Partial<OsExtraStatus>): OsExtraStatus => ({ id: 'gnome-shell', os: 'linux', kind: 'install', via: null, installed: false, available: true, reason: null, ...o })

test('row actions follow kind, availability and state', () => {
  assert.equal(rowAction(item({})), 'install')
  assert.equal(rowAction(item({ installed: true })), 'uninstall')
  assert.equal(rowAction(item({ available: false, reason: 'no-gnome' })), null)
  assert.equal(rowAction(item({ id: 'portals', kind: 'desktop' })), 'portals')
  assert.equal(rowAction(item({ kind: 'builtin', installed: true })), null)
  assert.equal(rowAction(item({ kind: 'delegated', via: 'desktop' })), null)
})

test('every catalog id has a label and reasons read as sentences', () => {
  for (const id of ['powertoys-run', 'windows-terminal', 'wsl', 'start-search', 'jump-list', 'widgets', 'shortcuts', 'spotlight', 'universal-clipboard', 'handoff', 'menu-bar', 'dock-menu', 'gnome-shell', 'plasmoid', 'xdg-scheme', 'portals', 'systemd-user', 'flatpak']) {
    assert.ok(LABELS[id], id)
  }
  assert.equal(reasonText(null), null)
  assert.match(reasonText('no-powertoys') ?? '', /PowerToys/)
  assert.equal(reasonText('unknown-code'), 'unknown-code')
})
