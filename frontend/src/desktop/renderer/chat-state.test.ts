// node --test src/desktop/renderer/chat-state.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { storeReducer, type ChatStore } from './chat-state.ts'

test('arka planda tamamlanan aracın kartı, sohbet açılınca kayıttaki aynı kimlikli araçla birleşir', () => {
  const id = 'sess-1'
  const at = 1_000
  let store: ChatStore = {}
  // Sohbet etkin değilken gelen canlı olaylar: store'da "partial" durum oluşur.
  for (const event of [
    { type: 'turnStart' },
    { type: 'tool', id: 'call-1', name: 'generate_image', summary: 'venice-sd35: kamp ateşi' },
    { type: 'toolResult', id: 'call-1', result: 'saved: C:\\x\\kor-media\\a.png' },
    { type: 'text', delta: 'Görsel hazır.' },
    { type: 'done' },
  ] as const) {
    store = storeReducer(store, { sessionId: id, action: { type: 'event', event, at } })
  }
  // Kayıt (Kor arka ucunun dökümü): aynı çağrı kimliği, farklı biçimde üretilmiş özet.
  store = storeReducer(store, {
    sessionId: id,
    action: {
      type: 'load',
      transcript: [
        { kind: 'user', text: 'logo üret', attachments: [] },
        { kind: 'tool', id: 'call-1', name: 'generate_image', summary: 'kamp ateşi logosu', result: 'saved: C:\\x\\kor-media\\a.png' },
        { kind: 'assistant', text: 'Görsel hazır.' },
      ],
    },
  })
  const tools = store[id]!.items.filter((i) => i.kind === 'tool')
  assert.equal(tools.length, 1, 'araç kartı bir kez görünür')
  assert.equal(tools[0]!.kind === 'tool' && tools[0]!.result, 'saved: C:\\x\\kor-media\\a.png')
  assert.equal(store[id]!.items.filter((i) => i.kind === 'assistant').length, 1)
})

test('yanıt akarken gelen düşünme parçası yanıtı bölmez', () => {
  const id = 'sess-2'
  let store: ChatStore = {}
  for (const event of [
    { type: 'turnStart' },
    { type: 'g3:reasoning', delta: 'Takvimi düşünüyorum.' },
    { type: 'text', delta: 'Go' },
    { type: 'g3:reasoning', delta: '' },
    { type: 'g3:reasoning', delta: '\n' },
    { type: 'g3:reasoning', delta: 'ek düşünce' },
    { type: 'text', delta: 'ogle Calendar' },
    { type: 'done' },
  ] as const) {
    store = storeReducer(store, { sessionId: id, action: { type: 'event', event, at: 1 } })
  }
  const items = store[id]!.items
  const answers = items.filter((i) => i.kind === 'assistant')
  assert.equal(answers.length, 1)
  assert.equal(answers[0]!.kind === 'assistant' && answers[0]!.text, 'Google Calendar')
  const kinds = items.map((i) => i.kind).filter((k) => k === 'thinking' || k === 'assistant')
  assert.deepEqual(kinds, ['thinking', 'assistant'], 'düşünme yanıtın önünde tek blok')
})
