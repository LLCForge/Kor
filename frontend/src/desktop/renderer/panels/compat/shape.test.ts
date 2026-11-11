import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { CompatSource } from '../../../ipc-compat.ts'
import { appliesText, ecoName, shortPath, toolSummary } from './shape.ts'

const src = (eco: string, path: string): { source: CompatSource } => ({ source: { eco, scope: 'project', path } })

test('toolSummary counts files and definitions per tool, Kor last', () => {
  const s = toolSummary(
    [src('kor', 'KOR.md'), src('cursor', '.cursor/rules/a.mdc'), src('cursor', '.cursor/rules/b.mdc'), src('claude', 'CLAUDE.md')],
    [src('claude', '.claude/settings.json'), src('claude', '.claude/settings.json'), src('cursor', '.cursor/mcp.json')],
  )
  assert.deepEqual(
    s.map((x) => [x.eco, x.files, x.rules, x.other]),
    [
      ['claude', 2, 1, 2],
      ['cursor', 3, 2, 1],
      ['kor', 1, 1, 0],
    ],
  )
  assert.deepEqual(toolSummary([], []), [])
})

test('appliesText, ecoName and shortPath', () => {
  assert.equal(appliesText({ kind: 'always' }), 'her zaman')
  assert.equal(appliesText({ kind: 'globs', globs: ['*.ts', 'src/**'] }), 'dosyalar: *.ts, src/**')
  assert.equal(appliesText({ kind: 'on_request', description: 'x' }), 'gerektiğinde')
  assert.equal(appliesText(null), '—')
  assert.equal(appliesText({ kind: 'future' }), '—')
  assert.equal(ecoName('windsurf'), 'Windsurf')
  assert.equal(ecoName('newtool'), 'newtool')
  assert.equal(shortPath('C:\\p\\.cursor\\rules\\a.mdc'), '.cursor/rules/a.mdc')
})
