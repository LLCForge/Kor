// node --test src/desktop/renderer/osint/batch-shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { commonDir, passSummary, previewRows, shortPath } from './batch-shape.ts'

test('paths are shortened under the common folder', () => {
  assert.equal(commonDir(['C:\\f\\a.jpg', 'c:\\F\\alt\\b.jpg']), 'C:/f')
  assert.equal(commonDir(['/a/x', '/b/y']), null)
  assert.equal(commonDir([]), null)
  assert.equal(shortPath('/home/u/foto/a.jpg', '/home/u/foto'), 'a.jpg')
  assert.equal(shortPath('/home/u/other.jpg', '/home/u/foto'), '/home/u/other.jpg')
  assert.equal(shortPath('/x/y', null), '/x/y')
})

test('preview rows carry labels and the in-place flag', () => {
  const rows = previewRows(
    [
      { op: 'rename', from: '/d/a b.jpg', to: '/d/a-b.jpg', detail: '' },
      { op: 'replace', from: '/d/n.txt', to: '/d/n.txt', detail: 'x → y' },
      { op: 'pdf_merge', from: '/d/1.pdf, /d/2.pdf', to: '/d/hepsi.pdf', detail: '' },
    ],
    '/d',
  )
  assert.deepEqual(rows.map((r) => [r.op, r.from, r.to, r.inPlace]), [
    ['Yeniden adlandır', 'a b.jpg', 'a-b.jpg', false],
    ['Bul ve değiştir', 'n.txt', 'n.txt', true],
    ['PDF birleştir', '1.pdf, 2.pdf', 'hepsi.pdf', false],
  ])
  assert.notEqual(rows[0]?.key, rows[1]?.key)
})

test('watcher passes are summarized only when something happened', () => {
  const empty = { sync: { written: [], linked: [], applied: [], kept: [], notes: [] }, tasks: [], inbox: 0 }
  assert.equal(passSummary(empty), null)
  assert.equal(
    passSummary({
      ...empty,
      sync: { ...empty.sync, written: ['a'] },
      tasks: [
        { status: 'done', result: 'r', file: 'f' },
        { status: 'failed', error: 'e', file: 'g' },
      ],
    }),
    '1 görünüm güncellendi, 1 görev tamamlandı, 1 görev başarısız',
  )
})
