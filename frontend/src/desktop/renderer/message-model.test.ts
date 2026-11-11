// §23.2 mesaj başlığı: yanıtı veren model (ADR 0053).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { modelAt, modelName } from './message-model.ts'

test('yanıtın modeli, ondan sonraki ilk model değişikliğinin from alanıdır', () => {
  const items = [
    { kind: 'user' },
    { kind: 'assistant' },
    { kind: 'modelNote', from: 'deepseek-v4', to: 'openai/gpt-5' },
    { kind: 'assistant' },
  ]
  assert.equal(modelAt(items, 1, 'openai/gpt-5'), 'deepseek-v4')
  assert.equal(modelAt(items, 3, 'openai/gpt-5'), 'openai/gpt-5')
})

test('sağlayıcı öneki model adından atılır', () => {
  assert.equal(modelName('openai/gpt-5', 'openai'), 'gpt-5')
  assert.equal(modelName('deepseek-v4', 'venice'), 'deepseek-v4')
})
