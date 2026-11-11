import assert from 'node:assert/strict'
import { test } from 'node:test'
import { toplam } from './sepet.ts'

test('indirimsiz toplam', () => {
  assert.equal(toplam([{ ad: 'kalem', fiyat: 10, adet: 2 }]), 20)
})
