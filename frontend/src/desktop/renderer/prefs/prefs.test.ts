// node --test src/desktop/renderer/prefs/prefs.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { emacsKey, initialEmacsState } from './emacs.ts'
import { composerAction, keyName, nextKeys } from './keymap.ts'
import { DEFAULT_UI, cssVars, diffMode, rootClasses, safeFontName } from './readability.ts'

const k = (key: string, ctrl = false, alt = false) => ({ key, ctrl, alt })
const ev = (key: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({ key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods })

test('readability values become CSS variables and classes', () => {
  const v = cssVars({ ...DEFAULT_UI, fontSize: 40, lineHeight: 1.8, maxWidth: 'narrow', codeFont: 'JetBrains Mono' })
  assert.equal(v['--kor-chat-font-size'], '24px')
  assert.equal(v['--kor-chat-line-height'], '1.8')
  assert.equal(v['--chat-width'], 'min(640px, 100%)')
  assert.equal(v['--kor-code-font'], "'JetBrains Mono', var(--font-mono)")
  assert.equal(cssVars({ ...DEFAULT_UI, codeFont: "x'; } body { display:none" })['--kor-code-font'], 'var(--font-mono)')
  assert.equal(safeFontName('  '), null)
  assert.deepEqual(rootClasses(DEFAULT_UI), ['kor-ui'])
  const all = rootClasses({ ...DEFAULT_UI, wrap: false, density: 'compact', screenReader: true, highContrast: true, toolOutput: 'quiet' })
  for (const c of ['kor-ui-nowrap', 'kor-ui-compact', 'kor-ui-still', 'kor-ui-contrast', 'kor-ui-reader', 'kor-ui-quiet']) assert.ok(all.includes(c), c)
  assert.equal(diffMode('split'), 'split')
  assert.equal(diffMode('inline'), 'unified')
})

test('emacs editing keys', () => {
  const t = 'hello world\nsecond line'
  assert.equal(emacsKey(initialEmacsState, k('a', true), t, 8).cursor, 0)
  assert.equal(emacsKey(initialEmacsState, k('e', true), t, 2).cursor, 11)
  assert.equal(emacsKey(initialEmacsState, k('f', false, true), t, 0).cursor, 5)
  assert.equal(emacsKey(initialEmacsState, k('b', false, true), t, 11).cursor, 6)
  assert.equal(emacsKey(initialEmacsState, k('n', true), t, 3).cursor, 15)
  assert.equal(emacsKey(initialEmacsState, k('p', true), t, 15).cursor, 3)
  const killed = emacsKey(initialEmacsState, k('k', true), t, 5)
  assert.equal(killed.text, 'hello\nsecond line')
  assert.equal(killed.state.kill, ' world')
  const yanked = emacsKey(killed.state, k('y', true), killed.text, 0)
  assert.equal(yanked.text, ' worldhello\nsecond line')
  assert.equal(emacsKey(initialEmacsState, k('k', true), 'ab\ncd', 2).text, 'abcd', 'at line end C-k joins lines')
  assert.equal(emacsKey(initialEmacsState, k('w', true), 'foo bar', 7).text, 'foo ')
  assert.equal(emacsKey(initialEmacsState, k('d', true), 'abc', 1).text, 'ac')
  assert.equal(emacsKey(initialEmacsState, k('h', true), 'abc', 1).text, 'bc')
  assert.equal(emacsKey(initialEmacsState, k('x'), 'abc', 1).handled, false)
})

test('composer key names and keymap actions', () => {
  assert.equal(keyName(ev('j', { ctrlKey: true })), 'ctrl+j')
  assert.equal(keyName(ev('Enter', { altKey: true })), 'alt+enter')
  assert.equal(keyName(ev('K', { shiftKey: true, ctrlKey: true })), 'ctrl+shift+k')
  assert.equal(keyName(ev('?', { shiftKey: true })), '?')
  assert.equal(keyName(ev('F5')), 'f5')
  assert.equal(keyName(ev('Dead')), null)
  const map = { 'ctrl+j': 'submit', 'alt+enter': 'newline', 'ctrl+x': 'fly' }
  assert.equal(composerAction(map, ev('j', { ctrlKey: true })), 'submit')
  assert.equal(composerAction(map, ev('x', { ctrlKey: true })), null)
  assert.equal(nextKeys('default'), 'vim')
  assert.equal(nextKeys('emacs'), 'default')
})
