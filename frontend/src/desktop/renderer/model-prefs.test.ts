// §23.2 model seçici: favoriler üstte, son 3 model hızlı erişimde, sağlayıcıya göre gruplama (ADR 0053).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { pushRecent, sectionModels, toggleFavorite } from './model-prefs.ts'

test('son kullanılanlar en çok 3, tekrarsız, en yeni başta', () => {
  let r: string[] = []
  for (const id of ['a', 'b', 'c', 'a', 'd']) r = pushRecent(r, id)
  assert.deepEqual(r, ['d', 'a', 'c'])
})

test('favori açılıp kapanır', () => {
  assert.deepEqual(toggleFavorite(['a'], 'b'), ['a', 'b'])
  assert.deepEqual(toggleFavorite(['a', 'b'], 'a'), ['b'])
})

test('bölümler: favoriler, son kullanılanlar, sonra sağlayıcı grupları; model bir kez görünür', () => {
  const models = [
    { id: 'openai/gpt-5', p: 'openai' },
    { id: 'deepseek-v4', p: 'venice' },
    { id: 'openai/gpt-5-mini', p: 'openai' },
    { id: 'qwen3', p: 'venice' },
    { id: 'anthropic/claude', p: 'anthropic' },
  ]
  const s = sectionModels(models, ['qwen3', 'missing'], ['openai/gpt-5', 'qwen3'], (m) => m.p, (p) => p.toUpperCase())
  assert.deepEqual(
    s.map((x) => [x.label, x.models.map((m) => m.id)]),
    [
      ['Favoriler', ['qwen3']],
      ['Son kullanılanlar', ['openai/gpt-5']],
      ['VENICE', ['deepseek-v4']],
      ['OPENAI', ['openai/gpt-5-mini']],
      ['ANTHROPIC', ['anthropic/claude']],
    ],
  )
})
