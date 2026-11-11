// node --test src/desktop/renderer/i18n/translate.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { compile } from './translate.ts'

function run(catalog: Record<string, string>, text: string): string {
  const c = compile(catalog)
  const hit = c.exact.get(text)
  if (hit) return hit
  for (const p of c.patterns) {
    const m = p.re.exec(text)
    if (m) {
      let i = 0
      const slots = new Map<string, string>()
      for (const name of p.key.match(/\{\d+\}/g) ?? []) slots.set(name, m[++i] ?? '')
      return p.out.replace(/\{\d+\}/g, (n) => slots.get(n) ?? n)
    }
  }
  return text
}

test('tam eşleşme ve kalıp', () => {
  const cat = { 'Yeni sohbet': 'New chat', '{0} dosya değişti': '{0} files changed', '{0} değişti': '{0} changed' }
  assert.equal(run(cat, 'Yeni sohbet'), 'New chat')
  assert.equal(run(cat, '3 dosya değişti'), '3 files changed')
  assert.equal(run(cat, 'model değişti'), 'model changed')
  assert.equal(run(cat, 'bilinmeyen'), 'bilinmeyen')
})

test('yer tutucu sırası değişebilir', () => {
  const cat = { '{0} / {1} tamamlandı': '{1} total, {0} done' }
  assert.equal(run(cat, '2 / 5 tamamlandı'), '5 total, 2 done')
})

test('harfsiz kalıp her şeyi yakalamaz, boş çeviri atlanır', () => {
  const c = compile({ '{0} · {1}': '{0} · {1}', 'Kaydet': '' })
  assert.equal(c.patterns.length, 0)
  assert.equal(c.exact.has('Kaydet'), false)
})
