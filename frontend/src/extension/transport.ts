// Eklenti ↔ Kor çekirdeği bağlantısı (§18.1, ADR 0058). Birincil yol Native Messaging (`kor chrome-native-host`,
// `kor chrome install` kaydeder): tarayıcı ana bilgisayarı başlatır, o da çalışan Kor'un köprüsüne aktarır; Kor
// kapalıysa ana bilgisayar bunu söyler ve tek tıkla başlatabilir. Ana bilgisayar kayıtlı değilse yedek yol yerel
// WebSocket'tir (ws://127.0.0.1:49201). İki yolda da ilk mesaj eşleştirmedir: kayıtlı anahtar ya da `kor chrome
// pair` kodu; eşleşmemiş bağlantıyı çekirdek kapatır.
// Uzak bağlantı (§18.4, §31.1): bu bilgisayarda Kor yoksa ve takım sunucusu ayarlıysa (sunucu, makine adı, üye
// belirteci) eklenti `wss://<sunucu>/api/v1/relay/dial/ext:<ad>` üzerinden o makinedeki Kor köprüsüne gider
// (`kor peer serve` orada dinler). Sunucu iletileri değiştirmeden aktarır; eşleştirme yine uçtan uca yapılır.

export const HOST_NAME = 'dev.kor.chrome_host'
export const WS_URL = 'ws://127.0.0.1:49201'

export interface Channel {
  send(text: string): void
  close(): void
}

export interface ChannelEvents {
  message(text: string): void
  /** `error`: kanal hiç açılamadı (ör. ana bilgisayar kayıtlı değil). */
  closed(error?: string): void
}

export interface Connectors {
  native(ev: ChannelEvents): Channel | null
  ws(url: string, ev: ChannelEvents, protocols?: string[]): Channel | null
}

export type Mode = 'native' | 'ws' | 'remote' | 'none'

/** Takım sunucusu üzerinden başka bir makinedeki Kor. */
export interface Remote {
  /** `https://kor.example.com[:port]` */
  server: string
  /** O makinenin adı (`kor peer list` / peers.toml `name`) */
  name: string
  /** Üye belirteci (`kor server add-member` paketi ya da `kor server login`) */
  token: string
}

/** Uzak köprü adresi; geçersiz ayarda `null` (yalnızca https sunucu, düz ad, düz belirteç). */
export function remoteUrl(r: Remote): string | null {
  let u: URL
  try {
    u = new URL(r.server.trim())
  } catch {
    return null
  }
  if (u.protocol !== 'https:' || u.username || u.password || !/^[A-Za-z0-9._-]{1,90}$/.test(r.name) || !/^[A-Za-z0-9_-]{8,200}$/.test(r.token)) return null
  return `wss://${u.host}/api/v1/relay/dial/ext:${r.name}`
}

/** Tarayıcı WebSocket'e başlık koyamaz: belirteç alt protokolde gider. */
export function remoteProtocols(r: Remote): string[] {
  return ['kor-relay', `bearer.${r.token}`]
}

export interface Status {
  mode: Mode
  /** Kor çekirdeği ulaşılabilir. */
  core: boolean
  paired: boolean
  /** Eşleştirme gerekiyor (anahtar yok ya da reddedildi). */
  needsPairing: boolean
  error: string
  /** Yalnızca native yolda: Kor'u başlatabilir. */
  canLaunch: boolean
}

export interface KeyStore {
  get(): Promise<string>
  set(key: string): Promise<void>
}

export class Transport {
  private ch: Channel | null = null
  private mode: Mode = 'none'
  private nativeMissing = false
  /** Yerel köprü yanıt vermedi: sıradaki denemede uzak yol (ayarlıysa). */
  private localMissing = false
  private pendingCode = ''
  private key = ''
  status: Status = { mode: 'none', core: false, paired: false, needsPairing: false, error: '', canLaunch: false }

  private readonly conn: Connectors
  private readonly keys: KeyStore
  private readonly onMessage: (msg: Record<string, unknown>) => void
  private readonly onStatus: (s: Status) => void
  private readonly remote: () => Promise<Remote | null>

  constructor(
    conn: Connectors,
    keys: KeyStore,
    onMessage: (msg: Record<string, unknown>) => void,
    onStatus: (s: Status) => void,
    remote: () => Promise<Remote | null> = async () => null,
  ) {
    this.conn = conn
    this.keys = keys
    this.onMessage = onMessage
    this.onStatus = onStatus
    this.remote = remote
  }

  private set(p: Partial<Status>): void {
    this.status = { ...this.status, ...p, mode: this.mode, canLaunch: this.mode === 'native' }
    this.onStatus(this.status)
  }

  connected(): boolean {
    return this.ch !== null && this.status.paired
  }

  /** Bağlı değilse bağlanır (önce native, olmazsa WebSocket) ve eşleştirme mesajını gönderir. */
  async connect(): Promise<void> {
    if (this.ch) return
    const key = await this.keys.get()
    this.key = key
    if (!key && !this.pendingCode) {
      this.set({ needsPairing: true, paired: false })
    }
    const events = (mode: Mode): ChannelEvents => ({
      message: (text) => this.receive(mode, text),
      closed: (error) => this.closed(mode, error),
    })
    let ch: Channel | null = null
    if (!this.nativeMissing) {
      this.mode = 'native'
      ch = this.conn.native(events('native'))
    }
    const remote = !ch && this.localMissing ? await this.remote() : null
    const url = remote ? remoteUrl(remote) : null
    if (!ch && remote && url) {
      this.mode = 'remote'
      ch = this.conn.ws(url, events('remote'), remoteProtocols(remote))
    }
    if (!ch) {
      this.mode = 'ws'
      ch = this.conn.ws(WS_URL, events('ws'))
    }
    if (!ch) {
      this.mode = 'none'
      this.set({ core: false, paired: false, error: 'Kor\'a bağlanılamadı' })
      return
    }
    this.ch = ch
    this.hello(key)
  }

  private hello(key: string): void {
    if (!this.ch) return
    if (this.pendingCode) this.ch.send(JSON.stringify({ type: 'pair', code: this.pendingCode }))
    else if (key) this.ch.send(JSON.stringify({ type: 'pair', key }))
    else if (this.mode === 'native') this.ch.send(JSON.stringify({ type: 'host', op: 'status' }))
  }

  /** `kor chrome pair` kodunu gönderir (gerekirse yeniden bağlanarak). */
  async pair(code: string): Promise<void> {
    this.pendingCode = code.trim()
    if (this.ch && this.status.paired) {
      this.close()
    }
    if (this.ch) {
      this.hello('')
      return
    }
    await this.connect()
  }

  /** Native ana bilgisayar üzerinden Kor masaüstünü başlatır. */
  launch(): boolean {
    if (this.mode !== 'native' || !this.ch) return false
    this.ch.send(JSON.stringify({ type: 'host', op: 'launch' }))
    return true
  }

  /** Düzenli yoklama (alarm): kopmuşsa yeniden bağlanır; native yolda Kor kapalıysa durumu sorar. */
  async poke(): Promise<void> {
    if (!this.ch) {
      await this.connect()
      return
    }
    if (this.mode === 'native' && !this.status.paired) this.ch.send(JSON.stringify({ type: 'host', op: 'status' }))
    else if (this.status.paired) this.ch.send(JSON.stringify({ type: 'ping' }))
  }

  send(msg: Record<string, unknown>): boolean {
    if (!this.ch || !this.status.paired) return false
    this.ch.send(JSON.stringify(msg))
    return true
  }

  close(): void {
    const ch = this.ch
    this.ch = null
    ch?.close()
  }

  private receive(mode: Mode, text: string): void {
    if (mode !== this.mode) return
    let msg: Record<string, unknown>
    try {
      msg = JSON.parse(text) as Record<string, unknown>
    } catch {
      return
    }
    if (msg.type === 'host') {
      const core = msg.core === true
      if (msg.op === 'status') {
        this.set({ core, paired: core && this.status.paired, error: typeof msg.error === 'string' ? msg.error : '' })
        // Kor yeni açıldı: ana bilgisayar köprüye bağlandı, şimdi eşleştirme.
        if (core && !this.status.paired && (this.key || this.pendingCode)) this.hello(this.key)
      }
      this.onMessage(msg)
      return
    }
    if (msg.type === 'paired') {
      this.pendingCode = ''
      const key = typeof msg.key === 'string' ? msg.key : ''
      if (key) {
        this.key = key
        void this.keys.set(key)
      }
      this.set({ core: true, paired: true, needsPairing: false, error: '' })
      this.onMessage(msg)
      return
    }
    if (msg.type === 'unpaired') {
      const hadCode = this.pendingCode !== ''
      this.pendingCode = ''
      if (!hadCode) {
        // Anahtar iptal edilmiş: yeniden eşleştirme gerekir.
        this.key = ''
        void this.keys.set('')
      }
      this.set({ core: true, paired: false, needsPairing: true, error: typeof msg.error === 'string' ? msg.error : 'Eşleştirme reddedildi' })
      return
    }
    this.onMessage(msg)
  }

  private closed(mode: Mode, error?: string): void {
    if (mode !== this.mode) return
    this.ch = null
    if (mode === 'native' && error && /not found|not registered|bulunamadı|No such native application/i.test(error)) {
      // Ana bilgisayar kayıtlı değil: bundan sonra doğrudan WebSocket.
      this.nativeMissing = true
    }
    // Yerel köprü yok (ya da uzak yol koptu): bir sonraki denemede öteki yol.
    if (mode === 'ws' && !this.status.paired) this.localMissing = true
    if (mode === 'remote') this.localMissing = false
    this.set({ core: false, paired: false, error: error ?? '' })
  }
}
