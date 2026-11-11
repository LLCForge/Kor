import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

// React kancaları her çizimde aynı sırada çağrılmalı: App'in erken dönüşlerinden (yükleniyor, API anahtarı)
// sonra kanca çağrılırsa ilk açılışta arayüz hiç çizilmez (temiz kurulumda boş pencere).
test('App bileşeninde erken dönüşlerden sonra React kancası çağrılmıyor', () => {
  const src = readFileSync(new URL('./App.tsx', import.meta.url), 'utf8').replace(/\r\n/g, '\n')
  const start = src.indexOf('export function App(')
  assert.ok(start >= 0, 'App bulunamadı')
  const firstReturn = src.indexOf('\n  if (', start + 1)
  const early = src.slice(firstReturn).search(/\n {2}if \([^\n]*\) return /)
  assert.ok(early >= 0, 'erken dönüş bulunamadı')
  const after = src.slice(firstReturn + early)
  const late = [...after.matchAll(/\n {2}(?:const [\w[\], ]+ = )?(use[A-Z]\w*)\(/g)].map((m) => m[1])
  assert.deepEqual(late, [], `erken dönüşten sonra kanca: ${late.join(', ')}`)
})
