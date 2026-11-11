// Tarayıcı eklenti API'leri (Chrome MV3 `chrome.*`, Firefox `browser.*`): yalnızca Kor eklentisinin kullandığı
// parçalar, söz (Promise) tabanlı. Firefox'ta olmayanlar (tabGroups, sidePanel, debugger) isteğe bağlıdır;
// çağıran taraf varlığını denetler. Masaüstü arayüzü bu dosyayı içermez (yalnızca eklenti paketleri).

export interface Tab {
  id?: number
  url?: string
  title?: string
  active: boolean
  windowId: number
  groupId?: number
  status?: string
}

export interface Port {
  name: string
  postMessage(message: unknown): void
  disconnect(): void
  onMessage: EventOf<(message: unknown) => void>
  onDisconnect: EventOf<(port: Port) => void>
}

export interface EventOf<F> {
  addListener(cb: F): void
  removeListener(cb: F): void
}

export interface InjectionResult<T> {
  result?: T
}

export interface WebRequestDetails {
  requestId: string
  tabId: number
  url: string
  method: string
  type: string
  statusCode?: number
  error?: string
}

export interface Ext {
  runtime: {
    id: string
    lastError?: { message?: string }
    getURL(path: string): string
    connect(info: { name: string }): Port
    connectNative(app: string): Port
    sendMessage(message: unknown): Promise<unknown>
    onConnect: EventOf<(port: Port) => void>
    onMessage: EventOf<(message: unknown, sender: { tab?: Tab }, respond: (r: unknown) => void) => boolean | undefined>
    onStartup: EventOf<() => void>
    onInstalled: EventOf<() => void>
  }
  tabs: {
    query(q: { active?: boolean; lastFocusedWindow?: boolean; currentWindow?: boolean; groupId?: number }): Promise<Tab[]>
    get(id: number): Promise<Tab>
    create(p: { url?: string; active?: boolean; windowId?: number }): Promise<Tab>
    update(id: number, p: { url?: string; active?: boolean }): Promise<Tab>
    remove(id: number): Promise<void>
    goBack?(id: number): Promise<void>
    goForward?(id: number): Promise<void>
    reload(id: number): Promise<void>
    captureVisibleTab(windowId: number | undefined, o: { format: 'png' | 'jpeg'; quality?: number }): Promise<string>
    group?(o: { tabIds: number[]; groupId?: number }): Promise<number>
    onUpdated: EventOf<(id: number, info: { status?: string; url?: string }, tab: Tab) => void>
    onActivated: EventOf<(info: { tabId: number }) => void>
    onRemoved: EventOf<(id: number) => void>
  }
  tabGroups?: {
    update(id: number, p: { title?: string; color?: string; collapsed?: boolean }): Promise<unknown>
  }
  scripting: {
    executeScript(o: {
      target: { tabId: number; frameIds?: number[] }
      func?: (...args: never) => unknown
      args?: unknown[]
      files?: string[]
      world?: 'ISOLATED' | 'MAIN'
      injectImmediately?: boolean
    }): Promise<Array<InjectionResult<unknown>>>
  }
  storage: {
    local: StorageArea
    session?: StorageArea
  }
  sidePanel?: {
    setPanelBehavior(b: { openPanelOnActionClick: boolean }): Promise<void>
    open(o: { tabId?: number; windowId?: number }): Promise<void>
  }
  sidebarAction?: { open(): Promise<void> }
  contextMenus?: {
    create(p: { id: string; title: string; contexts: string[] }): void
    removeAll(): Promise<void>
    onClicked: EventOf<(info: { menuItemId: string | number; selectionText?: string; pageUrl?: string }, tab?: Tab) => void>
  }
  webRequest?: {
    onBeforeRequest: EventOf<(d: WebRequestDetails) => void> & { addListener(cb: (d: WebRequestDetails) => void, f: { urls: string[] }): void }
    onCompleted: EventOf<(d: WebRequestDetails) => void> & { addListener(cb: (d: WebRequestDetails) => void, f: { urls: string[] }): void }
    onErrorOccurred: EventOf<(d: WebRequestDetails) => void> & { addListener(cb: (d: WebRequestDetails) => void, f: { urls: string[] }): void }
  }
  alarms?: {
    create(name: string, o: { periodInMinutes: number }): void
    onAlarm: EventOf<(a: { name: string }) => void>
  }
  action?: { setBadgeText(o: { text: string }): Promise<void> }
  permissions: {
    contains(p: { permissions: string[] }): Promise<boolean>
    request(p: { permissions: string[] }): Promise<boolean>
  }
  debugger?: {
    attach(t: { tabId: number }, version: string): Promise<void>
    detach(t: { tabId: number }): Promise<void>
    sendCommand(t: { tabId: number }, method: string, params?: object): Promise<unknown>
  }
  i18n: { getUILanguage(): string }
}

export interface StorageArea {
  get(keys: string | string[]): Promise<Record<string, unknown>>
  set(items: Record<string, unknown>): Promise<void>
  remove(keys: string | string[]): Promise<void>
}

/** Firefox `browser`, yoksa Chrome `chrome`. */
export function ext(): Ext {
  const g = globalThis as unknown as { browser?: Ext; chrome?: Ext }
  const e = g.browser ?? g.chrome
  if (!e) throw new Error('Eklenti API yok')
  return e
}

/** `storage.session` yoksa (eski Firefox) `local`. */
export function sessionStore(e: Ext): StorageArea {
  return e.storage.session ?? e.storage.local
}
