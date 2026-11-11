// §21.3: engellenen her kısayol, yönlendirmeler, Kor bağlam menüsü öğeleri ve gezinme politikası (ADR 0053).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { dropAllowed, keyDecision, menuItems, navDecision, nextScale, type KeyLike } from './webview-lock.ts'

const key = (k: string, mods: Partial<KeyLike> = {}): KeyLike => ({ key: k, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...mods })
const WIN = { editable: false, mac: false, devtools: false }
const MAC = { editable: false, mac: true, devtools: false }

test('spec §21.1 tarayıcı kısayollarının hepsi engellenir', () => {
  const blocked: [KeyLike, string][] = [
    [key('F12'), 'devtools'],
    [key('I', { ctrlKey: true, shiftKey: true }), 'devtools'],
    [key('J', { ctrlKey: true, shiftKey: true }), 'devtools'],
    [key('C', { ctrlKey: true, shiftKey: true }), 'devtools'],
    [key('u', { ctrlKey: true }), 'devtools'],
    [key('F5'), 'reload'],
    [key('r', { ctrlKey: true }), 'reload'],
    [key('R', { ctrlKey: true, shiftKey: true }), 'reload'],
    [key('ArrowLeft', { altKey: true }), 'history'],
    [key('ArrowRight', { altKey: true }), 'history'],
    [key('Backspace'), 'history'],
    [key('BrowserBack'), 'history'],
    [key('t', { ctrlKey: true }), 'tabs'],
    [key('n', { ctrlKey: true }), 'tabs'],
    [key('w', { ctrlKey: true }), 'tabs'],
    [key('T', { ctrlKey: true, shiftKey: true }), 'tabs'],
    [key('Tab', { ctrlKey: true }), 'tabs'],
    [key('F11'), 'browser'],
    [key('Delete', { ctrlKey: true, shiftKey: true }), 'browser'],
    [key('F3'), 'find-next'],
  ]
  for (const [k, reason] of blocked) assert.deepEqual(keyDecision(k, WIN), { kind: 'block', reason }, JSON.stringify(k))
})

test('macOS: Cmd birincil değiştirici; Cmd+Alt+I ve Cmd+[ engellenir', () => {
  assert.deepEqual(keyDecision(key('i', { metaKey: true, altKey: true }), MAC), { kind: 'block', reason: 'devtools' })
  assert.deepEqual(keyDecision(key('r', { metaKey: true }), MAC), { kind: 'block', reason: 'reload' })
  assert.deepEqual(keyDecision(key('[', { metaKey: true }), MAC), { kind: 'block', reason: 'history' })
  // macOS'ta Ctrl+R tarayıcı yenilemesi değildir.
  assert.deepEqual(keyDecision(key('r', { ctrlKey: true }), MAC), { kind: 'allow' })
})

test('yazdır/kaydet/bul/yakınlaştırma Kor eylemlerine yönlenir', () => {
  assert.deepEqual(keyDecision(key('p', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'export' })
  assert.deepEqual(keyDecision(key('s', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'save' })
  assert.deepEqual(keyDecision(key('f', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'find' })
  assert.deepEqual(keyDecision(key('+', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'zoomIn' })
  assert.deepEqual(keyDecision(key('=', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'zoomIn' })
  assert.deepEqual(keyDecision(key('-', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'zoomOut' })
  assert.deepEqual(keyDecision(key('0', { ctrlKey: true }), WIN), { kind: 'redirect', to: 'zoomReset' })
  assert.deepEqual(keyDecision({ ...key('Add', { ctrlKey: true }), code: 'NumpadAdd' }, WIN), { kind: 'redirect', to: 'zoomIn' })
})

test('düzenleme ve uygulama kısayolları serbest kalır', () => {
  const ed = { ...WIN, editable: true }
  for (const k of [key('Backspace'), key('ArrowLeft', { altKey: true }), key('C', { ctrlKey: true, shiftKey: true }), key('a', { ctrlKey: true }), key('z', { ctrlKey: true }), key('c', { ctrlKey: true }), key('v', { ctrlKey: true })]) {
    assert.deepEqual(keyDecision(k, ed), { kind: 'allow' }, JSON.stringify(k))
  }
  for (const k of [key('k', { ctrlKey: true }), key(',', { ctrlKey: true }), key('Enter'), key('Escape'), key('?'), key('j')]) {
    assert.deepEqual(keyDecision(k, WIN), { kind: 'allow' }, JSON.stringify(k))
  }
})

test('geliştirici build’inde devtools kısayolları açık', () => {
  const dev = { ...WIN, devtools: true }
  assert.deepEqual(keyDecision(key('F12'), dev), { kind: 'allow' })
  assert.deepEqual(keyDecision(key('I', { ctrlKey: true, shiftKey: true }), dev), { kind: 'allow' })
  // Yenileme geliştirici build'inde de kapalı (izlemede dist yeniden yüklenir; Rust gezinme kilidi aynı).
  assert.deepEqual(keyDecision(key('F5'), dev), { kind: 'block', reason: 'reload' })
})

test('Kor bağlam menüsü her bağlamda doğru öğelerle açılır', () => {
  const labels = (ctx: Parameters<typeof menuItems>[0], mac = false) => menuItems(ctx, mac).map((i) => i.label)
  assert.deepEqual(labels({ kind: 'editable', hasSelection: true, readOnly: false }), ['Kes', 'Kopyala', 'Yapıştır', 'Tümünü seç'])
  assert.deepEqual(labels({ kind: 'code', hasSelection: false }), ['Kopyala', 'Dosyaya kaydet', "Diff'e uygula"])
  assert.deepEqual(labels({ kind: 'message', role: 'assistant', hasSelection: false }), ['Alıntıla', 'Yeniden üret', 'Referans yap'])
  assert.deepEqual(labels({ kind: 'message', role: 'user', hasSelection: true }), ['Kopyala', 'Alıntıla', 'Referans yap'])
  assert.deepEqual(labels({ kind: 'image', src: 'x' }), ['Kaydet', 'Referans yap', 'Kopyala'])
  assert.deepEqual(labels({ kind: 'link', href: 'https://kor.dev', hasSelection: false }), ['Bağlantıyı tarayıcıda aç', 'Bağlantıyı kopyala'])
  assert.deepEqual(labels({ kind: 'selection' }), ['Kopyala'])
  assert.deepEqual(menuItems({ kind: 'none' }, false), [], 'boş zeminde menü yok (tarayıcı menüsü de yok)')
  const ed = menuItems({ kind: 'editable', hasSelection: false, readOnly: true }, true)
  assert.equal(ed.find((i) => i.action === 'cut')?.disabled, true)
  assert.equal(ed.find((i) => i.action === 'paste')?.disabled, true)
  assert.equal(ed.find((i) => i.action === 'copy')?.shortcut, '⌘C')
})

test('gezinme: uygulama içi serbest, http(s) sistem tarayıcısında, tehlikeli şemalar engelli', () => {
  const base = 'http://tauri.localhost/index.html'
  assert.equal(navDecision('#ust', base), 'allow')
  assert.equal(navDecision('./index.html', base), 'allow')
  assert.equal(navDecision('tauri://localhost/x', base), 'allow')
  assert.equal(navDecision('https://github.com/kor', base), 'external')
  assert.equal(navDecision('mailto:a@b.c', base), 'external')
  for (const bad of ['view-source:https://x', 'about:config', 'file:///C:/Windows/win.ini', 'javascript:alert(1)', 'data:text/html,x', 'chrome://settings', 'ftp://x']) {
    assert.equal(navDecision(bad, base), 'block', bad)
  }
})

test('Kor yazı boyutu adımları sınırlı', () => {
  assert.equal(nextScale(1, 'zoomIn'), 1.1)
  assert.equal(nextScale(1.5, 'zoomIn'), 1.5)
  assert.equal(nextScale(0.8, 'zoomOut'), 0.8)
  assert.equal(nextScale(1.25, 'zoomReset'), 1)
  assert.equal(nextScale(1.05, 'zoomOut'), 1)
})

test('dosya yalnızca izinli bölgelere bırakılır', () => {
  assert.equal(dropAllowed([{ dataset: {} }, { dataset: { korDrop: '' } }]), true)
  assert.equal(dropAllowed([{ dataset: {} }, {}]), false)
})
