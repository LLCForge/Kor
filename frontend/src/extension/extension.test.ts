// Kor tarayıcı eklentisi birim testleri (ADR 0058): `node --test src/extension/`.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { refusal, parseDomains, hostOf } from './policy.ts'
import { sensitive, hasPhrase, words } from './fields.ts'
import { Transport, type Channel, type ChannelEvents, type Status } from './transport.ts'
import { Groups } from './tabs.ts'
import { manifest, messages, CHROME_KEY } from './manifest.ts'
import { reduce, empty, answered, fromTranscript } from './panel/events.ts'
import { DICTS } from './panel/i18n.ts'
import { downloadName, fetchMedia } from './panel/media.ts'
import type { Ext, StorageArea, Tab } from './ext.ts'

test('domain policy matches subdomains, blocks deny and narrows to allow', () => {
  const d = parseDomains({ allow: [], deny: ['evil.com'] })
  assert.equal(refusal(d, 'https://any.site/x'), null)
  assert.match(refusal(d, 'https://shop.evil.com/') ?? '', /engelli/)
  assert.equal(refusal(d, 'https://notevil.com/'), null)
  const only = parseDomains({ allow: ['example.com'], deny: [] })
  assert.equal(refusal(only, 'https://www.example.com/'), null)
  assert.match(refusal(only, 'https://other.org/') ?? '', /izin listesinde değil/)
  assert.notEqual(refusal(only, 'file:///etc/passwd'), null)
  assert.equal(hostOf('chrome://settings'), null)
  assert.deepEqual(parseDomains(null), { allow: [], deny: [] })
})

test('sensitive fields are never written', () => {
  const f = (o: Partial<Parameters<typeof sensitive>[0]>) => sensitive({ type: 'text', autocomplete: '', label: '', name: '', id: '', placeholder: '', ...o })
  assert.equal(f({ type: 'password' }), true)
  assert.equal(f({ autocomplete: 'cc-number' }), true)
  assert.equal(f({ autocomplete: 'one-time-code' }), true)
  assert.equal(f({ label: 'Kart numarası' }), true)
  assert.equal(f({ name: 'card_number' }), true)
  assert.equal(f({ label: 'CVV' }), true)
  assert.equal(f({ label: 'PIN kodu' }), true)
  assert.equal(f({ label: 'Shipping address' }), false, 'pin inside a word is not a PIN')
  assert.equal(f({ label: 'E-posta', type: 'email' }), false)
  assert.equal(hasPhrase(words('Mot de passe'), 'mot de passe'), true)
})

class FakeChannel implements Channel {
  sent: string[] = []
  closed = false
  ev: ChannelEvents
  constructor(ev: ChannelEvents) {
    this.ev = ev
  }
  send(text: string): void {
    this.sent.push(text)
  }
  close(): void {
    this.closed = true
  }
}

function harness(native: boolean) {
  const store = { key: '' }
  const made: { native: FakeChannel[]; ws: FakeChannel[] } = { native: [], ws: [] }
  const statuses: Status[] = []
  const got: Array<Record<string, unknown>> = []
  const t = new Transport(
    {
      native: (ev) => {
        if (!native) return null
        const c = new FakeChannel(ev)
        made.native.push(c)
        return c
      },
      ws: (_url, ev) => {
        const c = new FakeChannel(ev)
        made.ws.push(c)
        return c
      },
    },
    { get: async () => store.key, set: async (k) => void (store.key = k) },
    (m) => got.push(m),
    (s) => statuses.push(s),
  )
  return { t, store, made, statuses, got }
}

test('transport pairs over native messaging, keeps the key and relays messages', async () => {
  const h = harness(true)
  await h.t.pair('ABCD-EFGH')
  const ch = h.made.native[0]
  assert.ok(ch)
  assert.deepEqual(JSON.parse(ch.sent[0]), { type: 'pair', code: 'ABCD-EFGH' })
  ch.ev.message(JSON.stringify({ type: 'paired', key: 'k'.repeat(64), policy: { allow: [], deny: [] } }))
  await Promise.resolve()
  assert.equal(h.store.key, 'k'.repeat(64))
  assert.equal(h.t.status.paired, true)
  assert.equal(h.t.status.canLaunch, true)
  assert.equal(h.t.send({ type: 'client', op: 'sessions' }), true)
  ch.ev.message(JSON.stringify({ type: 'event', event: { type: 'done' } }))
  assert.equal(h.got.at(-1)?.type, 'event')
  // Kor kapandı: ana bilgisayar söyler; yeniden açılınca anahtarla eşleşir.
  ch.ev.message(JSON.stringify({ type: 'host', op: 'status', core: false }))
  assert.equal(h.t.status.paired, false)
  ch.ev.message(JSON.stringify({ type: 'host', op: 'status', core: true }))
  assert.deepEqual(JSON.parse(ch.sent.at(-1) ?? '{}'), { type: 'pair', key: 'k'.repeat(64) })
  assert.equal(h.t.launch(), true)
})

test('transport falls back to WebSocket when the native host is not installed', async () => {
  const h = harness(true)
  h.store.key = 'x'.repeat(64)
  await h.t.connect()
  h.made.native[0].ev.closed('Specified native messaging host not found.')
  assert.equal(h.t.status.core, false)
  await h.t.poke()
  assert.equal(h.made.ws.length, 1)
  assert.deepEqual(JSON.parse(h.made.ws[0].sent[0]), { type: 'pair', key: 'x'.repeat(64) })
  // Anahtar iptal edilmişse silinir ve eşleştirme istenir.
  h.made.ws[0].ev.message(JSON.stringify({ type: 'unpaired', error: 'not paired' }))
  await Promise.resolve()
  assert.equal(h.store.key, '')
  assert.equal(h.t.status.needsPairing, true)
  assert.equal(h.t.launch(), false, 'only the native host can start Kor')
  assert.equal(h.t.send({ type: 'client' }), false, 'nothing is sent before pairing')
})

function fakeExt() {
  let nextTab = 10
  let nextGroup = 100
  const grouped: Array<{ tabIds: number[]; groupId?: number }> = []
  const titled: Array<{ id: number; title?: string; color?: string }> = []
  const data: Record<string, unknown> = {}
  const store: StorageArea = {
    get: async (k) => (typeof k === 'string' ? { [k]: data[k] } : {}),
    set: async (o) => void Object.assign(data, o),
    remove: async () => undefined,
  }
  const e = {
    tabs: {
      create: async (): Promise<Tab> => ({ id: nextTab++, active: true, windowId: 1 }),
      group: async (o: { tabIds: number[]; groupId?: number }) => {
        grouped.push(o)
        return o.groupId ?? nextGroup++
      },
    },
    tabGroups: { update: async (id: number, p: { title?: string; color?: string }) => void titled.push({ id, ...p }) },
  } as unknown as Ext
  return { e, store, grouped, titled }
}

test('each task gets its own ember tab group and owns only its tabs', async () => {
  const f = fakeExt()
  const g = new Groups(f.e, f.store)
  const a = { key: 'aaa', title: 'Kor · app' }
  const b = { key: 'bbb', title: 'Kor · web' }
  const t1 = await g.create(a, 'https://x.test', true)
  const t2 = await g.create(a, 'https://y.test', false)
  const t3 = await g.create(b, 'https://z.test', true)
  assert.deepEqual(await g.tabs(a), [t1, t2])
  assert.equal(await g.owns(a, t3), false)
  assert.equal(f.grouped[1].groupId, 100, 'second tab joins the first group')
  assert.deepEqual(f.titled[0], { id: 100, title: 'Kor · app', color: 'orange' })
  // Kullanıcının verdiği sekme son göreve girer.
  const got = await g.adopt(55)
  assert.equal(got.key, 'bbb')
  assert.equal(await g.owns(b, 55), true)
  await g.forget(t3)
  await g.forget(55)
  assert.equal(await g.ownerOf(55), null)
  // Kalıcı: yeni örnek aynı kaydı okur.
  assert.deepEqual(await new Groups(f.e, f.store).tabs(a), [t1, t2])
})

test('manifests: Chrome side panel and tab groups, Firefox sidebar and gecko id, native messaging in both', () => {
  const c = manifest('chrome', '1.2.3')
  const f = manifest('firefox', '1.2.3')
  for (const m of [c, f]) {
    assert.equal(m.manifest_version, 3)
    assert.ok((m.permissions as string[]).includes('nativeMessaging'))
    assert.ok((m.permissions as string[]).includes('scripting'))
    // Chrome's captureVisibleTab needs `<all_urls>`; http/https patterns are refused.
    assert.deepEqual(m.host_permissions, ['<all_urls>'])
  }
  assert.ok((c.permissions as string[]).includes('tabGroups') && (c.permissions as string[]).includes('sidePanel'))
  assert.deepEqual(c.optional_permissions, ['debugger'])
  assert.equal(c.key, CHROME_KEY)
  assert.deepEqual((f.browser_specific_settings as { gecko: { id: string } }).gecko.id, 'kor@kor.dev')
  assert.ok(f.sidebar_action && !f.side_panel && !(f.permissions as string[]).includes('tabGroups'))
  assert.deepEqual(f.background, { scripts: ['background.js'] })
  assert.equal(messages('fr').name.message, 'Kor')
})

test('agent events become panel rows like the desktop shows them', () => {
  let v = reduce(empty, { type: 'turnStart' })
  v = reduce(v, { type: 'text', delta: 'Mer' })
  v = reduce(v, { type: 'text', delta: 'haba' })
  v = reduce(v, { type: 'tool', id: 't1', name: 'browser_open', summary: 'https://x' })
  v = reduce(v, { type: 'tool', id: 't1', name: 'browser_open', summary: 'https://x' })
  v = reduce(v, { type: 'confirm', id: 't2', name: 'Write', summary: 'a.ts', files: [{ path: 'a.ts', before: null, after: 'x' }] })
  v = reduce(v, { type: 'agentStatus', status: { verb: 'Okuyor', elapsedMs: 3200 } })
  v = reduce(v, { type: 'g9:cost', turnUsd: 0.01234, live: true })
  assert.equal(v.items.length, 3)
  assert.deepEqual(v.items[0], { kind: 'assistant', text: 'Merhaba' })
  assert.equal(v.verb, 'Okuyor 3 sn')
  assert.equal(v.cost, '≈ $0.0123')
  v = answered(v, 't2')
  assert.equal(v.items.length, 2)
  v = reduce(v, { type: 'toolResult', id: 't1' })
  v = reduce(v, { type: 'done' })
  assert.equal(v.running, false)
  assert.deepEqual(v.items[1], { kind: 'tool', id: 't1', name: 'browser_open', summary: 'https://x', done: true })
  assert.deepEqual(fromTranscript([{ kind: 'user', text: 'hi' }, { kind: 'tool', id: 'x', name: 'Read', summary: 'a' }]).map((i) => i.kind), ['user', 'tool'])
})

test('every panel string has English and French', () => {
  const dir = resolve(dirname(fileURLToPath(import.meta.url)), 'panel')
  const missing: string[] = []
  for (const f of readdirSync(dir).filter((n) => n.endsWith('.tsx'))) {
    const s = readFileSync(resolve(dir, f), 'utf8')
    for (const m of s.matchAll(/\bt\('((?:[^'\\]|\\.)*)'\)/g)) {
      const tr = m[1].replace(/\\'/g, "'")
      for (const [lang, d] of Object.entries(DICTS)) if (!(tr in d)) missing.push(`${lang}: ${tr}`)
    }
  }
  assert.deepEqual(missing, [])
})

test('gallery media arrives in chunks and is joined into one blob', async () => {
  const file = new Uint8Array(1300).map((_, i) => i % 251)
  const asked: number[] = []
  const request = async (op: string, body: Record<string, unknown>) => {
    assert.equal(op, 'galleryChunk')
    assert.equal(body.id, 'cat.png')
    const offset = Number(body.offset)
    asked.push(offset)
    const part = file.subarray(offset, offset + 512)
    return { mime: 'image/png', size: file.length, offset, data: btoa(String.fromCharCode(...part)) }
  }
  const blob = await fetchMedia(request, 'cat.png')
  assert.deepEqual(asked, [0, 512, 1024])
  assert.equal(blob.type, 'image/png')
  assert.deepEqual(new Uint8Array(await blob.arrayBuffer()), file)
  // An empty file is one request; a file that shrinks while it is read is an error, not a loop.
  assert.equal((await fetchMedia(async () => ({ mime: 'audio/wav', size: 0, offset: 0, data: '' }), 'x.wav')).size, 0)
  await assert.rejects(fetchMedia(async () => ({ mime: 'video/mp4', size: 10, offset: 0, data: '' }), 'y.mp4'))
  assert.equal(downloadName('a/b\\c.png'), 'a_b_c.png')
})
