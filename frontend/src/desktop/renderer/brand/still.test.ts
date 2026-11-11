import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { stillSvg } from './still.ts'

test('stillSvg SMIL öğelerini çıkarır, çizimi korur', () => {
  for (const name of ['ember-pulse.svg', 'spinner-ember.svg', 'thinking-dots.svg', 'thinking-cube.svg']) {
    const svg = readFileSync(new URL(`../../../../../assets/${name}`, import.meta.url), 'utf8')
    const out = stillSvg(svg)
    assert.doesNotMatch(out, /<animate/, name)
    assert.match(out, /<\/svg>\s*$/, name)
  }
})
