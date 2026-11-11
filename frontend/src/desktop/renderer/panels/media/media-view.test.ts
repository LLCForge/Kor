// node --test src/desktop/renderer/panels/media/media-view.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { GalleryItem, MediaPlan } from '../../../ipc-media.ts'
import { cardFromPlan, costText, initialValues, missingFields, parseFree, presetFor, priceText } from './media-view.ts'

const model = {
  provider: 'venice',
  id: 'wan',
  name: 'wan',
  capabilities: ['video' as const],
  params: [
    { name: 'prompt', kind: 'string' as const, role: 'prompt', required: true },
    { name: 'resolution', kind: 'enum' as const, role: 'resolution', required: false, options: ['1080p', '720p'], default: '720p' },
    { name: 'aspect_ratio', kind: 'enum' as const, role: 'aspect_ratio', required: false, options: ['16:9', '9:16'], default: '16:9' },
  ],
  price: null,
  source: 'api.venice.ai/api/v1/models?type=video',
  acceptsReference: false,
}

const plan: MediaPlan = {
  model,
  plan: {
    input: { duration: '5s' },
    questions: [{ name: 'aspect_ratio', role: 'aspect_ratio', kind: 'enum', description: '', options: ['16:9', '9:16'], min: null, max: null, default: '16:9' }],
    unsupported: [{ name: 'resolution', asked: '4k', nearest: '1080p', allowed: '1080p, 720p' }],
    estimate: { amount: 0.4, currency: 'USD', estimated: false },
  },
  estimateText: '$0.40',
  alternatives: [],
  eta: 42,
  confirm: true,
  confirmAbove: 0.25,
}

test('the plan becomes one card: questions, unsupported values with the nearest as default, cost', () => {
  const card = cardFromPlan(plan, 'a cat')
  assert.deepEqual(card.fields.map((f) => [f.name, f.default]), [['aspect_ratio', '16:9'], ['resolution', '1080p']])
  assert.match(card.notes[0] ?? '', /4k.*1080p/)
  assert.equal(card.cost, '$0.40')
  assert.equal(card.eta, '≈ 42 sn')
  assert.equal(card.confirm, true)
  assert.deepEqual(card.fixed, { duration: '5s' })
  const v = initialValues(card)
  assert.deepEqual(missingFields(card, v), [])
})

test('free values respect ranges and options', () => {
  const f = { name: 'n', description: '', options: [], default: 1, min: 1, max: 4 }
  assert.equal(parseFree(f, ''), 1)
  assert.equal(parseFree(f, '3'), 3)
  assert.equal(parseFree(f, '9'), null)
  const e = { name: 'd', description: '', options: ['5s', '8s'], default: null, min: null, max: null }
  assert.equal(parseFree(e, '8S'), '8s')
  assert.equal(parseFree(e, '10s'), null)
})

const item: GalleryItem = {
  id: 'cat.png',
  file: 'kor-media/cat.png',
  kind: 'image',
  capability: 'image',
  provider: 'fal',
  model: 'fal-ai/flux/schnell',
  prompt: 'a cat',
  params: { prompt: 'a cat', seed: 7, image_size: 'square_hd', image_url: '(inline file)' },
  seed: 7,
  cost: 0.003,
  currency: 'USD',
  estimated: true,
  secs: 2,
  created: 0,
  inputs: [],
  parent: null,
  action: null,
}

test('quick actions keep the model and parameters or send the item as reference', () => {
  const v = presetFor(item, 'variation')
  assert.equal(v.model, 'fal:fal-ai/flux/schnell')
  assert.deepEqual(v.params, { image_size: 'square_hd' }, 'no seed, no files, no prompt')
  assert.equal(v.derivedFrom, 'cat.png')
  const r = presetFor(item, 'reseed')
  assert.equal(r.params['seed'], 7)
  assert.equal(r.prompt, '', 'a new prompt is asked for')
  const u = presetFor(item, 'upscale')
  assert.deepEqual([u.capability, u.reference, u.model], ['upscale', 'cat.png', null])
  assert.equal(presetFor(item, 'video').capability, 'image-to-video')
  assert.equal(costText(item), '≈$0.003')
  assert.equal(priceText({ amount: 0.03, currency: 'USD', unit: 'image' }), '$0.03/image')
  assert.equal(priceText(null), '?')
})
