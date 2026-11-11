// Görev sekme grupları (§18.2, ADR 0058): her Kor görevi kendi grubunu açar (ad: görev başlığı, renk: ember →
// Chrome'un "orange" grubu). Kor yalnızca kendi grubundaki sekmelerde işlem yapar; kullanıcının diğer sekmelerine
// dokunmaz. Kullanıcı bir sekmeyi yan panelden ("Bu sekmeyi Kor'a ver") son göreve verebilir. tabGroups API'si
// olmayan tarayıcılarda (Firefox) aynı yalıtım kayıt üzerinden yapılır, yalnızca görsel grup yoktur.
import type { Ext, StorageArea } from './ext.ts'

export interface Task {
  key: string
  title: string
}

interface Group {
  title: string
  groupId?: number
  tabs: number[]
}

const KEY = 'kor.groups'

export class Groups {
  private groups: Record<string, Group> = {}
  private loaded = false
  lastTask = ''

  private readonly e: Ext
  private readonly store: StorageArea

  constructor(e: Ext, store: StorageArea) {
    this.e = e
    this.store = store
  }

  private async load(): Promise<void> {
    if (this.loaded) return
    const v = (await this.store.get(KEY))[KEY] as { groups?: Record<string, Group>; last?: string } | undefined
    this.groups = v?.groups ?? {}
    this.lastTask = v?.last ?? ''
    this.loaded = true
  }

  private async save(): Promise<void> {
    await this.store.set({ [KEY]: { groups: this.groups, last: this.lastTask } })
  }

  /** Görevin grubunda yeni sekme açar. */
  async create(task: Task, url: string, active: boolean): Promise<number> {
    await this.load()
    const tab = await this.e.tabs.create({ url, active })
    if (tab.id === undefined) throw new Error('Sekme açılamadı')
    const g = (this.groups[task.key] ??= { title: task.title, tabs: [] })
    g.tabs.push(tab.id)
    this.lastTask = task.key
    await this.attach(g, tab.id)
    await this.save()
    return tab.id
  }

  private async attach(g: Group, tabId: number): Promise<void> {
    if (!this.e.tabs.group) return
    try {
      g.groupId = await this.e.tabs.group(g.groupId === undefined ? { tabIds: [tabId] } : { tabIds: [tabId], groupId: g.groupId })
    } catch {
      // Grup kapatılmış: yenisini aç.
      g.groupId = await this.e.tabs.group({ tabIds: [tabId] })
    }
    await this.e.tabGroups?.update(g.groupId, { title: g.title, color: 'orange' })
  }

  async owns(task: Task, tabId: number): Promise<boolean> {
    await this.load()
    return this.groups[task.key]?.tabs.includes(tabId) ?? false
  }

  /** Görevin sekmeleri (açılış sırasıyla). */
  async tabs(task: Task): Promise<number[]> {
    await this.load()
    return [...(this.groups[task.key]?.tabs ?? [])]
  }

  /** Bütün görevlerin sekmeleri (hizmet çalışanı yeniden başlayınca ağ kaydı için). */
  async all(): Promise<number[]> {
    await this.load()
    return Object.values(this.groups).flatMap((g) => g.tabs)
  }

  /** Sekme kapandı (kullanıcı ya da Kor). */
  async forget(tabId: number): Promise<void> {
    await this.load()
    for (const [k, g] of Object.entries(this.groups)) {
      g.tabs = g.tabs.filter((t) => t !== tabId)
      if (g.tabs.length === 0) delete this.groups[k]
    }
    await this.save()
  }

  /** Kullanıcının izniyle bir sekmeyi göreve verir (yan panel); görev yoksa "Kor" grubu açılır. */
  async adopt(tabId: number, task?: Task): Promise<Task> {
    await this.load()
    const key = task?.key ?? (this.lastTask || 'user')
    const g = (this.groups[key] ??= { title: task?.title ?? 'Kor', tabs: [] })
    if (!g.tabs.includes(tabId)) g.tabs.push(tabId)
    this.lastTask = key
    await this.attach(g, tabId)
    await this.save()
    return { key, title: g.title }
  }

  /** Sekme hangi göreve ait (yoksa `null`). */
  async ownerOf(tabId: number): Promise<string | null> {
    await this.load()
    return Object.entries(this.groups).find(([, g]) => g.tabs.includes(tabId))?.[0] ?? null
  }
}
