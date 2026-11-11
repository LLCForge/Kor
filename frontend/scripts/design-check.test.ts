// ADR 0053: tek tema kaynağı (§16.2 kural 1) ve serbest değer yasağı (§23.5).
import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { test } from 'node:test'
import { ROOT, THEME_PATH, loadTheme, patchTauri, renderCss, renderRust, staleOutputs, validateTheme, type Theme } from './theme-gen.ts'
import { rawValues, readAllowlist, scanRawValues, scanThemeCopies, themeCopies, themeNeedles } from './design-check.ts'

const theme = JSON.parse(readFileSync(THEME_PATH, 'utf8')) as Theme

test('theme.json şeması geçerli ve spec renklerini taşıyor', () => {
  assert.deepEqual(validateTheme(theme), [])
  assert.equal(theme.ember.mid, '#F8A21F')
  assert.equal(theme.ink.bg, '#14151B')
  assert.deepEqual(theme.status, { success: '#9BE564', warning: '#FFD842', error: '#FF5C5C', info: '#7FB2FF' })
  assert.deepEqual(validateTheme({ ...theme, gradient: ['#zz'] }).length > 0, true)
})

test('üretilen dosyalar theme.json ile eşzamanlı', () => {
  assert.deepEqual(staleOutputs(), [], 'node scripts/theme-gen.ts çalıştırın')
})

test('CSS dört yüzey katmanını, kenarlığı ve hareket token’larını üretir', () => {
  const css = renderCss(theme)
  for (const v of ['--kor-layer-bg: #14151B', '--kor-layer-surface: #1C1D23', '--kor-layer-raised: #242529', '--kor-layer-overlay: #34353C', '--kor-border: rgb(255 255 255 / 0.08)', '--kor-dur-hover: 150ms', '--kor-ease: cubic-bezier(.2,.8,.2,1)', '--kor-radius-composer: 14px', '--kor-space-7: 48px']) {
    assert.ok(css.includes(v), v)
  }
  assert.match(css, /prefers-reduced-motion: reduce[\s\S]*--kor-dur-panel: 0ms/)
})

test('Rust sabitleri RGB üçlüsü olarak üretilir ve kendi eşzamanlılık testini taşır', () => {
  const rs = renderRust(theme)
  assert.match(rs, /pub const MID: Rgb = \(248, 162, 31\);/)
  assert.match(rs, /fn in_sync_with_theme_json\(\)/)
})

test('Tauri yaması yalnızca pencere arka planını değiştirir', () => {
  const conf = JSON.stringify({ app: { windows: [{ label: 'main' }], security: { csp: { a: 1 } } } })
  const out = JSON.parse(patchTauri(conf, theme)) as { app: { windows: { backgroundColor: string }[]; security: unknown } }
  assert.equal(out.app.windows[0]!.backgroundColor, theme.ink.bg)
  assert.deepEqual(out.app.security, { csp: { a: 1 } })
  assert.equal(patchTauri(patchTauri(conf, theme), theme), patchTauri(conf, theme))
})

test('elle kopyalanmış tema hex’i ve Rust RGB üçlüsü yakalanır', () => {
  const n = themeNeedles(theme)
  assert.equal(themeCopies('color: #f8a21f;', 'x.css', n).length, 1)
  assert.equal(themeCopies('const C: (u8,u8,u8) = (248, 162, 31);', 'x.rs', n).length, 1)
  assert.equal(themeCopies('"\\x1b[38;2;248;162;31m"', 'x.rs', n).length, 1)
  assert.equal(themeCopies('color: #123456;', 'x.css', n).length, 0)
})

test('depoda tema rengi kopyası yok (assets/theme.json tek kaynak)', () => {
  assert.deepEqual(scanThemeCopies(), [])
})

test('serbest hex/px değerleri yakalanır; 1px, 0 ve token serbest', () => {
  assert.deepEqual(rawValues('a { padding: 13px; color: #fff }', 'a.css').map((v) => v.value), ['#fff', '13px'])
  assert.deepEqual(rawValues('a { border: 1px solid var(--kor-border); margin: 0px; gap: var(--kor-space-2) }', 'a.css'), [])
  assert.deepEqual(rawValues('/* 13px yorum */ a { }', 'a.css'), [])
  assert.deepEqual(rawValues('<a href="#main">x</a>', 'a.tsx'), [])
})

test('allowlist dışındaki arayüz dosyalarında serbest değer yok ve liste yalnızca var olan dosyaları içerir', () => {
  assert.deepEqual(scanRawValues(), [])
  for (const f of readAllowlist()) assert.ok(existsSync(resolve(ROOT, f)), `${f} silinmiş: allowlist'ten çıkarın`)
  assert.ok(loadTheme().verbs.tr.includes('Közlüyor'))
})
