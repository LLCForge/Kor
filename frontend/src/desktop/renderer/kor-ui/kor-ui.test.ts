// kor-ui (spec §23.5, ADR 0053): saf mantık + bileşen kütüphanesinin bütünlüğü.
import assert from 'node:assert/strict'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { MAX_TOASTS, TOOLTIP_DELAY_MS, createToastStore, cx, segmentedNext } from './logic.ts'

const DIR = dirname(fileURLToPath(import.meta.url))
const COMPONENTS = ['Button', 'Chip', 'Select', 'Tooltip', 'Card', 'Segmented', 'Toast', 'Modal', 'CommandPalette']

test('spec’teki dokuz bileşen var, index dışa aktarıyor ve her birinin Ladle hikâyesi var', () => {
  const index = readFileSync(resolve(DIR, 'index.ts'), 'utf8')
  for (const c of COMPONENTS) {
    assert.ok(existsSync(resolve(DIR, `${c}.tsx`)), c)
    assert.match(index, new RegExp(`from './${c}.tsx'`), c)
    assert.ok(existsSync(resolve(DIR, 'stories', `${c}.stories.tsx`)), `${c} hikâyesi`)
  }
})

test('hikâyeler birden çok durumu belgeliyor', () => {
  for (const f of readdirSync(resolve(DIR, 'stories')).filter((f) => f.endsWith('.stories.tsx'))) {
    const n = [...readFileSync(resolve(DIR, 'stories', f), 'utf8').matchAll(/^export const \w+/gm)].length
    assert.ok(n >= 2, `${f}: ${n} durum`)
  }
})

test('yalnız ikonlu butonlar etiketli (aria-label + title)', () => {
  for (const f of readdirSync(DIR).filter((f) => f.endsWith('.tsx'))) {
    const src = readFileSync(resolve(DIR, f), 'utf8')
    for (const m of src.matchAll(/<button[^>]*>\s*<(X|[A-Z]\w+) size=/g)) {
      const tag = m[0]
      assert.match(tag, /aria-label=/, `${f}: ${tag}`)
      assert.match(tag, /title=/, `${f}: ${tag}`)
    }
  }
})

test('tooltip gecikmesi 400 ms', () => {
  assert.equal(TOOLTIP_DELAY_MS, 400)
})

test('segment kontrolü ok tuşlarıyla dolaşır, devre dışıyı atlar, uçlara gider', () => {
  const opts = [{ value: 'a' }, { value: 'b', disabled: true }, { value: 'c' }] as const
  assert.equal(segmentedNext(opts, 'a', 'ArrowRight'), 'c')
  assert.equal(segmentedNext(opts, 'c', 'ArrowRight'), 'a')
  assert.equal(segmentedNext(opts, 'a', 'ArrowLeft'), 'c')
  assert.equal(segmentedNext(opts, 'c', 'Home'), 'a')
  assert.equal(segmentedNext(opts, 'a', 'End'), 'c')
  assert.equal(segmentedNext(opts, 'a', 'Enter'), null)
  assert.equal(segmentedNext([{ value: 'x', disabled: true }], 'x', 'ArrowRight'), null)
})

test('bildirim kuyruğu: en çok üç, hatalar kalıcı, kapatma ve abonelik', () => {
  const s = createToastStore()
  let calls = 0
  const off = s.subscribe(() => calls++)
  const ids = ['a', 'b', 'c', 'd'].map((t) => s.push(t))
  assert.equal(s.list().length, MAX_TOASTS)
  assert.deepEqual(s.list().map((t) => t.text), ['b', 'c', 'd'])
  const err = s.push('x'.repeat(500), 'error')
  assert.equal(s.list().at(-1)!.timeout, 0)
  assert.equal(s.list().at(-1)!.text.length, 400)
  s.dismiss(err)
  s.dismiss(9999)
  assert.ok(!s.list().some((t) => t.id === err))
  assert.equal(s.list().find((t) => t.id === ids[3])!.timeout, 4000)
  off()
  const before = calls
  s.push('y')
  assert.equal(calls, before)
})

test('cx boşları atlar', () => {
  assert.equal(cx('a', false, null, undefined, 'b'), 'a b')
})
