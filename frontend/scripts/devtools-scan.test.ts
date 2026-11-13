// node --test scripts/devtools-scan.test.ts
// The §21.3 binary scan against fake binaries: a release-like one passes, each devtools marker fails.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { MARKERS, scan } from './devtools-scan.ts'

const SCRIPT = fileURLToPath(new URL('./devtools-scan.ts', import.meta.url))

/** Random bytes around `parts`, like strings in a binary's data section. */
function fake(...parts: string[]): Buffer {
  const noise = () => Buffer.from(Array.from({ length: 4096 }, (_, i) => (i * 131 + 7) % 251))
  return Buffer.concat(parts.flatMap((p) => [noise(), Buffer.from(p, 'latin1')]).concat(noise()))
}

// Strings every build has (ACL of core:default, the WKWebView preference key, our own lock code).
const RELEASE = fake('plugin:webview|internal_toggle_devtools\0', 'internal_toggle_devtools', 'developerExtrasEnabled\0', 'KOR_DEVTOOLS', '--devtools', 'is_inspector_open')

function run(file: string): { code: number | null; out: string } {
  const r = spawnSync(process.execPath, [SCRIPT, file], { encoding: 'utf8' })
  return { code: r.status, out: r.stdout + r.stderr }
}

test('a release-like binary has no marker', () => {
  assert.deepEqual(scan(RELEASE), [])
})

test('each marker is found', () => {
  for (const m of MARKERS) assert.deepEqual(scan(fake('x', m.text)), [m], m.why)
})

test('the macOS selector needs its NUL bounds', () => {
  assert.deepEqual(scan(fake('set_inspector_open\0', '\0_inspectorFoo\0')), [])
})

test('the command line: exit 0 clean, 1 with markers, 2 unreadable', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kor-devtools-scan-'))
  const clean = join(dir, 'kor-desktop')
  const dirty = join(dir, 'kor-desktop-debug')
  writeFileSync(clean, RELEASE)
  writeFileSync(dirty, fake('function toggleDevtoolsHotkey() {}'))
  assert.equal(run(clean).code, 0)
  const bad = run(dirty)
  assert.equal(bad.code, 1)
  assert.match(bad.out, /toggleDevtoolsHotkey/)
  assert.equal(run(join(dir, 'missing')).code, 2)
})
