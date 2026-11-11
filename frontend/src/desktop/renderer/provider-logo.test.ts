// §23.2 sağlayıcı logoları: yerel, OpenRouter, monogram ve assets/providers bütünlüğü (ADR 0053).
import assert from 'node:assert/strict'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { isLocalProvider, logoPlan, monogramOf } from './provider-logo.ts'
import { renderGen, scanLogos } from '../../../scripts/provider-logos.ts'

const ROOT = resolve(fileURLToPath(import.meta.url), '../../../../..')
const LOGOS = resolve(ROOT, 'assets/providers')
const known = new Set(['anthropic', 'deepseek', 'qwen', 'openrouter'])
const has = (p: string) => known.has(p)

test('yerel sağlayıcılar yerel ikonu alır', () => {
  assert.ok(isLocalProvider('local-lmstudio') && isLocalProvider('ollama') && isLocalProvider('local'))
  assert.equal(logoPlan('local-vllm', 'qwen3', has).kind, 'local')
})

test('OpenRouter modelinde asıl sağlayıcının logosu + OpenRouter rozeti', () => {
  const p = logoPlan('openrouter', 'deepseek/deepseek-v4', has)
  assert.deepEqual([p.kind, p.provider, p.via, p.modelId], ['brand', 'deepseek', 'openrouter', 'deepseek-v4'])
  assert.equal(logoPlan('openrouter', 'auto', has).via, null)
})

test('bilinmeyen sağlayıcı ember çemberde harf monogramı', () => {
  const p = logoPlan('dashscope', 'x', has)
  assert.equal(p.kind, 'monogram')
  assert.equal(p.monogram, 'D')
  assert.equal(monogramOf('ışık'), 'I')
  assert.equal(monogramOf('-'), '?')
})

test('assets/providers: her logo tek renk (currentColor), renkli varyant ve lisans notu var', () => {
  const files = readdirSync(LOGOS)
  const logos = scanLogos(files)
  assert.ok(logos.length >= 30)
  for (const l of logos) {
    const mono = readFileSync(resolve(LOGOS, `${l.id}.svg`), 'utf8')
    assert.match(mono, /currentColor/, `${l.id}.svg tek renk değil`)
    assert.match(mono, /viewBox="0 0 24 24"/, `${l.id}.svg 24×24 değil`)
  }
  assert.ok(existsSync(resolve(LOGOS, 'LICENSES.md')))
  const gen = readFileSync(resolve(ROOT, 'frontend/src/desktop/renderer/provider-logos.gen.ts'), 'utf8').replace(/\r\n/g, '\n')
  assert.equal(gen, renderGen(logos), 'node scripts/provider-logos.ts çalıştırın')
})
