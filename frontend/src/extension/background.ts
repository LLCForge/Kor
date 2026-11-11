// Kor eklentisi arka planı (Chrome MV3 hizmet çalışanı, Firefox olay sayfası) — ADR 0058.
// - Kor çekirdeğine bağlanır (transport.ts: önce Native Messaging, yoksa yerel WebSocket; eşleştirme anahtarıyla).
// - Çekirdeğin tarayıcı aracı komutlarını görev sekme gruplarında yürütür (tabs.ts); grubun dışındaki sekmelere
//   dokunmaz, alan adı listesini (policy.ts) her işlemden önce denetler, parola/kart alanlarına yazmaz (page/index.ts).
// - Yan panelle (panel/) uzun ömürlü bağlantı: Kor olayları panele, panelin istekleri çekirdeğe.
// - Seçili metin için sağ tık "Kor: bunu uygula" (§27.11) ve hosting paneli bağlamı (§29.2) burada.
// Sayfa içeriği yalnızca veridir; burada hiçbir sayfa metni komut olarak yorumlanmaz.
import { ext, sessionStore, type Port, type Tab, type WebRequestDetails } from './ext.ts'
import { Transport, HOST_NAME, type Channel, type ChannelEvents, type Status } from './transport.ts'
import { Groups, type Task } from './tabs.ts'
import { refusal, parseDomains, type Domains } from './policy.ts'

const e = ext()
const store = sessionStore(e)
const groups = new Groups(e, store)
const panels = new Set<Port>()
let domains: Domains = { allow: [], deny: [] }
const watched = new Set<number>()
const KEY = 'kor.key'

function toPanels(m: Record<string, unknown>): void {
  for (const p of panels) {
    try {
      p.postMessage(m)
    } catch {
      panels.delete(p)
    }
  }
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

// --- bağlantı -------------------------------------------------------------------------------------------------
const connectors = {
  native(ev: ChannelEvents): Channel | null {
    try {
      const port = e.runtime.connectNative(HOST_NAME)
      port.onMessage.addListener((m) => ev.message(JSON.stringify(m)))
      port.onDisconnect.addListener(() => ev.closed(e.runtime.lastError?.message ?? 'native host closed'))
      return { send: (t) => port.postMessage(JSON.parse(t)), close: () => port.disconnect() }
    } catch {
      return null
    }
  },
  ws(url: string, ev: ChannelEvents): Channel | null {
    let ws: WebSocket
    try {
      ws = new WebSocket(url)
    } catch {
      return null
    }
    const queue: string[] = []
    ws.onopen = () => {
      for (const t of queue.splice(0)) ws.send(t)
    }
    ws.onmessage = (m) => ev.message(String(m.data))
    ws.onclose = () => ev.closed()
    return {
      send: (t) => (ws.readyState === WebSocket.OPEN ? ws.send(t) : queue.push(t)),
      close: () => ws.close(),
    }
  },
}

const keys = {
  get: async () => {
    const v = (await e.storage.local.get(KEY))[KEY]
    return typeof v === 'string' ? v : ''
  },
  set: (k: string) => e.storage.local.set({ [KEY]: k }),
}

const transport = new Transport(connectors, keys, onCore, (s: Status) => {
  void e.action?.setBadgeText({ text: s.paired ? 'ON' : '' })
  toPanels({ kind: 'status', status: s })
})

// Çekirdeğe giden yan panel istekleri: rid → yanıt bekleyen.
const waiters = new Map<number, (r: unknown) => void>()
let rid = 0

function request(type: 'client' | 'panel', body: Record<string, unknown>, timeoutMs: number): Promise<unknown> {
  return new Promise((resolve) => {
    const id = ++rid
    const timer = setTimeout(() => {
      waiters.delete(id)
      resolve({ error: 'Kor yanıt vermedi' })
    }, timeoutMs)
    waiters.set(id, (r) => {
      clearTimeout(timer)
      resolve(r)
    })
    const lang = (e.i18n.getUILanguage() || 'tr').slice(0, 2)
    if (!transport.send({ ...body, type, rid: id, lang })) {
      clearTimeout(timer)
      waiters.delete(id)
      resolve({ error: 'Kor\'a bağlı değil. Kurulumdan eşleştirin ya da Kor\'u başlatın.' })
    }
  })
}

function onCore(msg: Record<string, unknown>): void {
  if (msg.type === 'paired') {
    domains = parseDomains(msg.policy)
    return
  }
  if (msg.type === 'event') {
    toPanels({ kind: 'event', event: msg.event })
    return
  }
  if (msg.type === 'clientReply' || msg.type === 'panelReply') {
    const w = waiters.get(Number(msg.rid))
    if (w) {
      waiters.delete(Number(msg.rid))
      w(msg.result)
    }
    return
  }
  if (msg.type === 'host') {
    toPanels({ kind: 'host', host: msg })
    return
  }
  if (typeof msg.id === 'number' && typeof msg.cmd === 'string') {
    void command(msg).then(
      (result) => transport.send({ id: msg.id, ok: true, result }),
      (err: unknown) => transport.send({ id: msg.id, ok: false, error: err instanceof Error ? err.message : String(err) }),
    )
  }
}

// --- sayfa betikleri ------------------------------------------------------------------------------------------
type PageResult = { ok: true; [k: string]: unknown } | { ok: false; error: string }

async function callPage(tabId: number, name: string, args: unknown[] = []): Promise<Record<string, unknown>> {
  await e.scripting.executeScript({ target: { tabId }, files: ['page.js'] })
  const [r] = await e.scripting.executeScript({
    target: { tabId },
    func: (n: string, a: unknown[]) => {
      const page = (globalThis as unknown as { __korPage: Record<string, (...x: unknown[]) => unknown> }).__korPage
      return page[n](...a)
    },
    args: [name, args],
  })
  const res = r?.result as PageResult | undefined
  if (!res) throw new Error('Sayfa yanıt vermedi (tarayıcı sayfası ya da izin verilmeyen site olabilir)')
  if (!res.ok) throw new Error(res.error)
  return res
}

async function mainWorld<T>(tabId: number, func: (...a: never) => T, args: unknown[] = []): Promise<T> {
  const [r] = await e.scripting.executeScript({ target: { tabId }, world: 'MAIN', func, args })
  return r?.result as T
}

async function hookConsole(tabId: number): Promise<void> {
  try {
    await e.scripting.executeScript({ target: { tabId }, files: ['console.js'], world: 'MAIN', injectImmediately: true })
  } catch {
    // tarayıcının kendi sayfaları
  }
}

function waitComplete(tabId: number, ms = 25000): Promise<void> {
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer)
      e.tabs.onUpdated.removeListener(listener)
      resolve()
    }
    const timer = setTimeout(done, ms)
    const listener = (id: number, info: { status?: string }) => {
      if (id === tabId && info.status === 'complete') done()
    }
    e.tabs.onUpdated.addListener(listener)
    void e.tabs.get(tabId).then((t) => t.status === 'complete' && done(), done)
  })
}

// --- ağ kaydı (yalnızca Kor sekmeleri) ----------------------------------------------------------------------------
interface NetEntry {
  id: string
  method: string
  url: string
  type: string
  status?: number
  error?: string
}
const net = new Map<number, NetEntry[]>()

function record(d: WebRequestDetails, patch: Partial<NetEntry>): void {
  if (!watched.has(d.tabId)) return
  const list = net.get(d.tabId) ?? []
  let entry = list.find((x) => x.id === d.requestId)
  if (!entry) {
    entry = { id: d.requestId, method: d.method, url: d.url.slice(0, 500), type: d.type }
    list.push(entry)
    if (list.length > 300) list.splice(0, list.length - 300)
  }
  Object.assign(entry, patch)
  net.set(d.tabId, list)
}
e.webRequest?.onBeforeRequest.addListener((d) => record(d, {}), { urls: ['<all_urls>'] })
e.webRequest?.onCompleted.addListener((d) => record(d, { status: d.statusCode }), { urls: ['<all_urls>'] })
e.webRequest?.onErrorOccurred.addListener((d) => record(d, { error: d.error }), { urls: ['<all_urls>'] })

// --- ekran görüntüleri ------------------------------------------------------------------------------------------
async function debuggerAllowed(): Promise<boolean> {
  return !!e.debugger && (await e.permissions.contains({ permissions: ['debugger'] }).catch(() => false))
}

async function withDebugger<T>(tabId: number, f: (send: (m: string, p?: object) => Promise<unknown>) => Promise<T>): Promise<T> {
  const d = e.debugger
  if (!d) throw new Error('debugger yok')
  await d.attach({ tabId }, '1.3')
  try {
    return await f((m, p) => d.sendCommand({ tabId }, m, p))
  } finally {
    await d.detach({ tabId }).catch(() => undefined)
  }
}

async function blobToDataUrl(b: Blob): Promise<string> {
  const bytes = new Uint8Array(await b.arrayBuffer())
  let bin = ''
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return `data:${b.type};base64,${btoa(bin)}`
}

async function bitmap(dataUrl: string): Promise<ImageBitmap> {
  return createImageBitmap(await (await fetch(dataUrl)).blob())
}

async function visible(tab: Tab): Promise<string> {
  if (!tab.active && tab.id !== undefined) {
    await e.tabs.update(tab.id, { active: true })
    await sleep(350)
  }
  return e.tabs.captureVisibleTab(tab.windowId, { format: 'png' })
}

async function screenshot(tabId: number, full: boolean): Promise<string> {
  if (await debuggerAllowed()) {
    const r = (await withDebugger(tabId, (send) => send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: full }))) as { data: string }
    return `data:image/png;base64,${r.data}`
  }
  const tab = await e.tabs.get(tabId)
  const first = await visible(tab)
  if (!full) return first
  // Tam sayfa: kaydırıp parça parça yakalar (captureVisibleTab saniyede en çok iki kez), en çok 10 ekran.
  const m = await mainWorld(tabId, () => ({ h: document.documentElement.scrollHeight, vh: innerHeight, w: innerWidth, dpr: devicePixelRatio, y: scrollY }))
  const steps = Math.min(10, Math.ceil(m.h / m.vh))
  const canvas = new OffscreenCanvas(Math.round(m.w * m.dpr), Math.round(Math.min(m.h, steps * m.vh) * m.dpr))
  const ctx = canvas.getContext('2d')
  if (!ctx) return first
  for (let i = 0; i < steps; i++) {
    const y = await mainWorld(tabId, (to: number) => {
      scrollTo(0, to)
      return scrollY
    }, [i * m.vh])
    await sleep(550)
    const img = await bitmap(await e.tabs.captureVisibleTab(tab.windowId, { format: 'png' }))
    ctx.drawImage(img, 0, Math.round(y * m.dpr))
  }
  await mainWorld(tabId, (to: number) => scrollTo(0, to), [m.y])
  return blobToDataUrl(await canvas.convertToBlob({ type: 'image/png' }))
}

async function crop(dataUrl: string, r: { x: number; y: number; w: number; h: number }, dpr: number): Promise<string> {
  const img = await bitmap(dataUrl)
  const w = Math.max(1, Math.round(r.w * dpr))
  const h = Math.max(1, Math.round(r.h * dpr))
  const canvas = new OffscreenCanvas(w, h)
  canvas.getContext('2d')?.drawImage(img, Math.round(r.x * dpr), Math.round(r.y * dpr), w, h, 0, 0, w, h)
  return blobToDataUrl(await canvas.convertToBlob({ type: 'image/png' }))
}

// --- çekirdeğin tarayıcı komutları ------------------------------------------------------------------------------
function checkUrl(url: string): void {
  const why = refusal(domains, url)
  if (why) throw new Error(why)
}

async function owned(task: Task, tabId: unknown): Promise<number> {
  const id = Number(tabId)
  if (!Number.isInteger(id) || !(await groups.owns(task, id))) {
    throw new Error('Bu sekme Kor\'un görev grubunda değil; Kor yalnızca kendi grubundaki sekmelerde çalışır (kullanıcı yan panelden bir sekmeyi verebilir).')
  }
  const tab = await e.tabs.get(id)
  if (tab.url && /^https?:/.test(tab.url)) checkUrl(tab.url)
  return id
}

async function command(m: Record<string, unknown>): Promise<unknown> {
  const task = (m.task ?? { key: 'user', title: 'Kor' }) as Task
  switch (m.cmd) {
    case 'tabs.create': {
      const url = String(m.url ?? 'about:blank')
      if (url !== 'about:blank') checkUrl(url)
      const id = await groups.create(task, url, m.active !== false)
      watched.add(id)
      await hookConsole(id)
      await waitComplete(id)
      return { tabId: id }
    }
    case 'tabs.list': {
      const out = []
      for (const id of await groups.tabs(task)) {
        try {
          const t = await e.tabs.get(id)
          out.push({ tabId: id, url: t.url ?? '', title: t.title ?? '', active: t.active })
        } catch {
          await groups.forget(id)
        }
      }
      return out
    }
    case 'tabs.close': {
      const id = await owned(task, m.tabId)
      await e.tabs.remove(id)
      await groups.forget(id)
      watched.delete(id)
      return null
    }
    case 'tabs.select': {
      const id = await owned(task, m.tabId)
      await e.tabs.update(id, { active: true })
      return null
    }
    case 'navigate': {
      const id = await owned(task, m.tabId)
      const url = String(m.url)
      checkUrl(url)
      await e.tabs.update(id, { url })
      await sleep(150)
      await waitComplete(id)
      const t = await e.tabs.get(id)
      return { url: t.url, title: t.title }
    }
    case 'history': {
      const id = await owned(task, m.tabId)
      if (m.action === 'reload') await e.tabs.reload(id)
      else if (m.action === 'forward') await (e.tabs.goForward ? e.tabs.goForward(id) : mainWorld(id, () => history.forward()))
      else await (e.tabs.goBack ? e.tabs.goBack(id) : mainWorld(id, () => history.back()))
      await sleep(300)
      await waitComplete(id)
      return { url: (await e.tabs.get(id)).url }
    }
    case 'snapshot': {
      const id = await owned(task, m.tabId)
      return (await callPage(id, 'snapshot')).snapshot
    }
    case 'act': {
      const id = await owned(task, m.tabId)
      return callPage(id, 'act', [String(m.action), m])
    }
    case 'console': {
      const id = await owned(task, m.tabId)
      await hookConsole(id)
      return mainWorld(id, (clear: boolean) => {
        const w = window as unknown as { __korConsole?: string[] }
        const lines = [...(w.__korConsole ?? [])]
        if (clear) w.__korConsole?.splice(0)
        return lines
      }, [m.clear === true])
    }
    case 'network': {
      const id = await owned(task, m.tabId)
      const list = [...(net.get(id) ?? [])]
      if (m.clear === true) net.delete(id)
      return list
    }
    case 'screenshot': {
      const id = await owned(task, m.tabId)
      return { dataUrl: await screenshot(id, m.fullPage === true), url: (await e.tabs.get(id)).url }
    }
    case 'eval': {
      const id = await owned(task, m.tabId)
      const code = String(m.code ?? '')
      if (await debuggerAllowed()) {
        const r = (await withDebugger(id, (send) => send('Runtime.evaluate', { expression: code, returnByValue: true, awaitPromise: true }))) as {
          result?: { value?: unknown }
          exceptionDetails?: { text?: string }
        }
        if (r.exceptionDetails) throw new Error(r.exceptionDetails.text ?? 'script error')
        return r.result?.value ?? null
      }
      // Hata ayıklayıcı izni yoksa sayfanın kendi dünyasında; sayfanın CSP'si eval'i engelleyebilir.
      return mainWorld(id, (c: string) => {
        try {
          return { value: JSON.parse(JSON.stringify((0, eval)(c) ?? null)) as unknown }
        } catch (err) {
          return { error: String(err) }
        }
      }, [code]).then((r) => {
        if (r.error !== undefined) throw new Error(`${r.error} (sayfanın güvenlik politikası engellemiş olabilir; yan panelden hata ayıklayıcı iznini verin)`)
        return r.value
      })
    }
    case 'frames': {
      const id = await owned(task, m.tabId)
      const tab = await e.tabs.get(id)
      const count = Math.min(50, Math.max(2, Number(m.count) || 10))
      const interval = Math.min(3000, Math.max(550, Number(m.intervalMs) || 600))
      const frames: string[] = []
      for (let i = 0; i < count; i++) {
        if (i > 0) await sleep(interval)
        frames.push(i === 0 ? await visible(tab) : await e.tabs.captureVisibleTab(tab.windowId, { format: 'jpeg', quality: 70 }))
      }
      return frames
    }
    default:
      throw new Error(`Bilinmeyen komut: ${String(m.cmd)}`)
  }
}

// --- yan panel ----------------------------------------------------------------------------------------------
async function activeTab(): Promise<Tab> {
  const [tab] = await e.tabs.query({ active: true, lastFocusedWindow: true })
  if (!tab || tab.id === undefined) throw new Error('Etkin sekme yok')
  return tab
}

async function activeWeb(): Promise<Tab & { id: number; url: string }> {
  const tab = await activeTab()
  if (!tab.url || !/^https?:/.test(tab.url)) throw new Error('Bu sayfa bir web sayfası değil')
  checkUrl(tab.url)
  return tab as Tab & { id: number; url: string }
}

async function recordStep(label: string): Promise<void> {
  let shot = ''
  try {
    shot = await e.tabs.captureVisibleTab(undefined, { format: 'jpeg', quality: 50 })
  } catch {
    // ekran görüntüsü alınamazsa adım yine kaydedilir
  }
  const steps = ((await store.get('steps')).steps as unknown[] | undefined) ?? []
  const step = { at: Date.now(), label, shot }
  await store.set({ steps: [...steps, step].slice(-20) })
  toPanels({ kind: 'step', step })
}

async function panelOp(op: string, m: Record<string, unknown>): Promise<unknown> {
  switch (op) {
    case 'status':
      await transport.poke()
      return transport.status
    case 'pair':
      await transport.pair(String(m.code ?? ''))
      return transport.status
    case 'launch':
      if (!transport.launch()) throw new Error('Kor\'u başlatmak için yerel ana bilgisayar gerekli: kor chrome install')
      return null
    case 'client':
      return request('client', (m.body ?? {}) as Record<string, unknown>, Number(m.timeoutMs) || 300000)
    case 'panel':
      return request('panel', (m.body ?? {}) as Record<string, unknown>, 180000)
    case 'activeTab': {
      const t = await activeTab()
      return { url: t.url ?? '', title: t.title ?? '' }
    }
    case 'pageText': {
      const t = await activeWeb()
      return callPage(t.id, 'pageText')
    }
    case 'screenshot': {
      const t = await activeWeb()
      return { dataUrl: await e.tabs.captureVisibleTab(t.windowId, { format: 'png' }), url: t.url, title: t.title }
    }
    case 'adopt': {
      const t = await activeWeb()
      const task = await groups.adopt(t.id)
      watched.add(t.id)
      await hookConsole(t.id)
      return task
    }
    case 'formRead':
      return callPage((await activeWeb()).id, 'readForm')
    case 'formFill':
      return callPage((await activeWeb()).id, 'fillForm', [m.fills])
    case 'formValues':
      return callPage((await activeWeb()).id, 'formValues', [m.refs])
    case 'pick': {
      const t = await activeWeb()
      const cap = await callPage(t.id, 'pick')
      const shot = await e.tabs.captureVisibleTab(t.windowId, { format: 'png' })
      const rect = cap.rect as { x: number; y: number; w: number; h: number }
      return { ...cap, screenshot: await crop(shot, rect, Number(cap.dpr) || 1), width: Math.round(rect.w), height: Math.round(rect.h) }
    }
    case 'panelFill': {
      const t = await activeWeb()
      const done = await callPage(t.id, 'panelFill', [m.fields])
      const filled = done.filled as string[]
      const missing = done.missing as string[]
      await recordStep(`Form dolduruldu: ${filled.join(', ')}${missing.length ? ` (bulunamadı: ${missing.join(', ')})` : ''}`)
      return done
    }
    case 'recordStep':
      await recordStep(String(m.label ?? ''))
      return null
    case 'steps':
      return ((await store.get('steps')).steps as unknown[] | undefined) ?? []
    case 'pending': {
      const v = (await store.get('pending')).pending ?? null
      await store.remove('pending')
      return v
    }
    case 'debugger':
      return debuggerAllowed()
    default:
      throw new Error(`Bilinmeyen istek: ${op}`)
  }
}

e.runtime.onConnect.addListener((port) => {
  if (port.name !== 'kor-panel') return
  panels.add(port)
  void transport.connect()
  port.postMessage({ kind: 'status', status: transport.status })
  port.onMessage.addListener((raw) => {
    const m = (raw ?? {}) as Record<string, unknown>
    const id = m.rid
    void panelOp(String(m.op), m).then(
      (result) => port.postMessage({ kind: 'res', rid: id, ok: true, result }),
      (err: unknown) => port.postMessage({ kind: 'res', rid: id, ok: false, error: err instanceof Error ? err.message : String(err) }),
    )
  })
  port.onDisconnect.addListener(() => panels.delete(port))
})

// Sekme değişince panel bağlamı yeniler (hosting paneli tanıma, form/dokümantasyon araçları).
e.tabs.onActivated.addListener(() => toPanels({ kind: 'tabChanged' }))
e.tabs.onUpdated.addListener((id, info, tab) => {
  if (info.status === 'loading' && watched.has(id)) void hookConsole(id)
  if (info.status === 'complete' && tab.active) toPanels({ kind: 'tabChanged' })
})
e.tabs.onRemoved.addListener((id) => {
  watched.delete(id)
  net.delete(id)
  void groups.forget(id)
})

// §27.11: seçili metin → "Kor: bunu uygula".
function menus(): void {
  if (!e.contextMenus) return
  void e.contextMenus.removeAll().then(() =>
    e.contextMenus?.create({ id: 'kor-apply', title: menuTitle(), contexts: ['selection'] }),
  )
}

function menuTitle(): string {
  const lang = (e.i18n.getUILanguage() || 'tr').slice(0, 2)
  return lang === 'fr' ? 'Kor : appliquer ceci' : lang === 'en' ? 'Kor: apply this' : 'Kor: bunu uygula'
}

e.contextMenus?.onClicked.addListener((info, tab) => {
  if (info.menuItemId !== 'kor-apply') return
  void store.set({ pending: { kind: 'apply', selection: info.selectionText ?? '', url: info.pageUrl ?? tab?.url ?? '', title: tab?.title ?? '' } })
  // Panel açıksa hemen, değilse açılınca `pending` ile alır.
  toPanels({ kind: 'pending' })
  if (e.sidePanel && tab) void e.sidePanel.open({ windowId: tab.windowId }).catch(() => undefined)
  else void e.sidebarAction?.open().catch(() => undefined)
})

// MV3 hizmet çalışanı boşta sonlandırılır: alarm bağlantıyı yoklar (trafik de ömrü uzatır).
e.alarms?.create('kor-keepalive', { periodInMinutes: 0.4 })
e.alarms?.onAlarm.addListener(() => void transport.poke())
e.runtime.onStartup.addListener(() => void transport.connect())
e.runtime.onInstalled.addListener(() => {
  menus()
  void transport.connect()
})
void groups.all().then((ids) => ids.forEach((id) => watched.add(id)))
void e.sidePanel?.setPanelBehavior({ openPanelOnActionClick: true }).catch(() => undefined)
void transport.connect()
