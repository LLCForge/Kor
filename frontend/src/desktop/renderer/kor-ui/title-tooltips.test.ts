import assert from 'node:assert/strict'
import { test } from 'node:test'
import { pairLabel, placeTip } from './title-tooltips.ts'

test('simge düğmesinin eksik adı diğerinden doldurulur', () => {
  assert.deepEqual(pairLabel('Kapat', null, null), { title: 'Kapat' })
  assert.deepEqual(pairLabel(null, 'Yenile', null), { aria: 'Yenile' })
  // İpucu açıkken title data-kor-title'da: geri yazılmaz.
  assert.deepEqual(pairLabel('Ara', null, 'Ara'), {})
  assert.deepEqual(pairLabel('A', 'B', null), {})
  assert.deepEqual(pairLabel(' ', null, null), {})
})

test('ipucu öğenin altında ortalanır, sığmazsa üste geçer ve pencerede kalır', () => {
  const view = { width: 800, height: 600 }
  assert.deepEqual(placeTip({ left: 100, top: 100, width: 40, height: 20 }, { width: 60, height: 24 }, view), { left: 90, top: 126 })
  // Alt kenara yakın: üstte açılır.
  assert.equal(placeTip({ left: 100, top: 580, width: 40, height: 20 }, { width: 60, height: 24 }, view).top, 580 - 24 - 6)
  // Sağ kenara yakın: içeri kayar.
  assert.equal(placeTip({ left: 790, top: 10, width: 10, height: 10 }, { width: 120, height: 24 }, view).left, 800 - 120 - 8)
  // Sol kenar.
  assert.equal(placeTip({ left: 0, top: 10, width: 10, height: 10 }, { width: 120, height: 24 }, view).left, 8)
})
