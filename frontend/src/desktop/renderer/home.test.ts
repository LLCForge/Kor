// §23.3 karşılama: selam, bağlama duyarlı çipler, sürüm rozeti (ADR 0053).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { greeting, homeChips, projectName, releaseLabel } from './home.ts'

test('selam günün saatine göre değişir ve proje adını söyler', () => {
  assert.equal(greeting(8, null), 'Günaydın')
  assert.equal(greeting(14, null), 'İyi günler')
  assert.equal(greeting(20, 'kor-agent'), 'İyi akşamlar — kor-agent')
  assert.equal(greeting(2, null), 'İyi geceler')
  assert.equal(greeting(23, null), 'İyi geceler')
})

test('proje adı yolun son bileşeni', () => {
  assert.equal(projectName('C:\\Users\\user\\kor-agent\\'), 'kor-agent')
  assert.equal(projectName('/home/x/site'), 'site')
  assert.equal(projectName(null), null)
})

test('proje yoksa yalnız klasör aç ve örnek proje', () => {
  assert.deepEqual(homeChips({ root: null, isRepo: null, dirty: 0, conflicts: 0, securityFindings: null }).map((c) => c.id), ['openFolder', 'sample'])
})

test('proje varsa analiz/git sinyalleri önce gelir, en çok 5 çip', () => {
  const chips = homeChips({ root: '/p', isRepo: true, dirty: 2, conflicts: 1, securityFindings: 3 })
  assert.deepEqual(chips.map((c) => c.label), ['1 çakışmayı çöz', '3 güvenlik bulgusunu incele', '2 değişikliği gözden geçir', "Son commit'i açıkla", 'Projeyi açıkla'])
  const clean = homeChips({ root: '/p', isRepo: false, dirty: 0, conflicts: 0, securityFindings: 0 })
  assert.deepEqual(clean.map((c) => c.id), ['explain', 'bugs', 'tests', 'improve'])
  assert.ok(chips.every((c) => c.prompt))
})

test('sürüm rozeti gerçek sürüm ve kanaldır; bilinmiyorsa gösterilmez', () => {
  assert.equal(releaseLabel('0.8.0', 'beta'), '0.8.0 · beta')
  assert.equal(releaseLabel('0.8.0', 'stable'), '0.8.0')
  assert.equal(releaseLabel(null, 'beta'), null)
})
