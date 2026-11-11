// node --test src/desktop/renderer/g5/template-inputs.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { TemplateInput } from '../../ipc-templates.ts'
import { buildTemplateInputs } from './template-inputs.ts'

const input = (name: string, kind: TemplateInput['kind'], required = false): TemplateInput => ({ name, kind, required, default: null, description: null })

test('form values become typed template inputs', () => {
  const inputs = [input('name', { type: 'string' }, true), input('count', { type: 'number' }), input('auth', { type: 'bool' }), input('method', { type: 'enum', values: ['GET', 'POST'] }), input('dir', { type: 'path' })]
  assert.deepEqual(buildTemplateInputs(inputs, { name: ' users ', count: '3', auth: false, method: '', dir: '' }), { name: 'users', count: 3, auth: false })
  // Bad numbers are passed through as text so the core reports the validation error.
  assert.deepEqual(buildTemplateInputs(inputs, { count: 'x', unknown: 'y' }), { count: 'x' })
  assert.deepEqual(buildTemplateInputs(inputs, {}), {})
})
