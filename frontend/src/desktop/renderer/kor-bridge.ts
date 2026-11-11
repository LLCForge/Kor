// window.kor: arayüzün tek IPC yüzeyi. Her çağrı Rust tarafındaki tek `kor_call` komutuna gider:
//   kor.git.status(a, b)  ->  invoke('kor_call', { method: 'git.status', args: [a, b] })
// `onX(listener)` abonelikleri Tauri olaylarına bağlanır (Rust bu adlarla yayar):
//   kor.onEvent -> 'kor:event'   kor.g4.onEvent -> 'kor:g4/event'   kor.onSessionsChanged -> 'kor:sessionsChanged'
import { invoke } from '@tauri-apps/api/core'
import { listen, type Event } from '@tauri-apps/api/event'
import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview'
import type { KorApi } from '../ipc.ts'
import { recordEvent, recordIpc } from './perfmon/stats.ts'

const SUBSCRIPTION = /^on[A-Z]/

export function eventName(path: readonly string[]): string {
  const last = path[path.length - 1] ?? ''
  const name = last.charAt(2).toLowerCase() + last.slice(3)
  return `kor:${[...path.slice(0, -1), name].join('/')}`
}

function subscribe(path: readonly string[]) {
  return (listener: (payload: unknown) => void): (() => void) => {
    let stopped = false
    let unlisten: (() => void) | null = null
    void listen(eventName(path), (e) => {
      recordEvent()
      listener(e.payload)
    }).then((fn) => {
      // Abonelik dinleyici kaydı tamamlanmadan kaldırıldıysa hemen bırak.
      if (stopped) fn()
      else unlisten = fn
    })
    return () => {
      stopped = true
      unlisten?.()
    }
  }
}

/** İkili veri JSON'da `{ $bytes: base64 }` olarak gelir (ör. g21ReadDoc); arayüz Uint8Array bekler. */
export function decodeBytes(value: unknown): unknown {
  if (value === null || typeof value !== 'object' || !('$bytes' in value)) return value
  const b64 = (value as { $bytes: unknown }).$bytes
  if (typeof b64 !== 'string') return value
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

/**
 * Büyük yükü olan yöntemler JSON yerine ham bayt döner (§32.2, ADR 0062; Rust `ipc_bytes.rs`):
 * `text` UTF-8 metne çözülür (sözleşme aynı kalır), `bytes` Uint8Array olarak verilir.
 */
export const BINARY: Readonly<Record<string, 'text' | 'bytes'>> = { readFile: 'text', g21ReadDoc: 'bytes' }

/** `kor_bytes` yanıtı (ArrayBuffer) → sözleşmedeki tür. */
export function decodeBinary(kind: 'text' | 'bytes', buf: ArrayBuffer | Uint8Array): string | Uint8Array {
  const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf)
  return kind === 'text' ? new TextDecoder().decode(bytes) : bytes
}

async function call(method: string, args: unknown[]): Promise<unknown> {
  const started = performance.now()
  try {
    const binary = BINARY[method]
    if (binary) return decodeBinary(binary, await invoke<ArrayBuffer>('kor_bytes', { method, args }))
    // Performans bindirmesi ve ölçüm kendi komutunda (Rust perfmon.rs, ADR 0062).
    if (method.startsWith('perfmon.')) return await invoke('kor_perf', { method, args })
    return decodeBytes(await invoke('kor_call', { method, args }))
  } catch (err) {
    // Rust `Err(String)` düz metin olarak gelir; arayüz `err.message` bekler.
    throw err instanceof Error ? err : new Error(typeof err === 'string' ? err : JSON.stringify(err))
  } finally {
    recordIpc(method, performance.now() - started)
  }
}

// Webview'daki File nesnesinin yolu yoktur. İşletim sisteminden sürüklenen dosyaları Tauri kendisi yakalar
// (HTML drop'a dosya gelmez); yollar `onNativeDrop` ile alınır. Bu yalnızca sayfa içi sürüklemeler için kalır.
const SYNC_ROOT: Record<string, unknown> = { g2PathForFile: () => '' }

export interface NativeDrop {
  type: 'over' | 'drop' | 'leave'
  /** Yalnızca `drop`'ta dolu: mutlak dosya yolları. */
  paths: string[]
  /** Pencere içi CSS pikseli. */
  x: number
  y: number
}

/** İşletim sisteminden pencereye sürüklenen dosyalar (Tauri sürükle-bırak olayı). */
export function onNativeDrop(listener: (d: NativeDrop) => void): () => void {
  let stopped = false
  let unlisten: (() => void) | null = null
  void getCurrentWebview()
    .onDragDropEvent((e: Event<DragDropEvent>) => {
      const p = e.payload
      if (p.type === 'leave') {
        listener({ type: 'leave', paths: [], x: 0, y: 0 })
        return
      }
      const scale = window.devicePixelRatio || 1
      listener({ type: p.type === 'drop' ? 'drop' : 'over', paths: p.type === 'drop' ? p.paths : [], x: p.position.x / scale, y: p.position.y / scale })
    })
    .then((fn) => {
      if (stopped) fn()
      else unlisten = fn
    })
  return () => {
    stopped = true
    unlisten?.()
  }
}

function node(path: readonly string[]): unknown {
  const children = new Map<string, unknown>()
  const fn = (...args: unknown[]) => call(path.join('.'), args)
  return new Proxy(fn, {
    get(_target, key) {
      // `then` tanımsız kalmalı: aksi halde nesne thenable sanılır ve await'te askıda kalır.
      if (typeof key !== 'string' || key === 'then') return undefined
      if (path.length === 0 && key in SYNC_ROOT) return SYNC_ROOT[key]
      let child = children.get(key)
      if (child === undefined) {
        const next = [...path, key]
        child = SUBSCRIPTION.test(key) ? subscribe(next) : node(next)
        children.set(key, child)
      }
      return child
    },
  })
}

export function createKorBridge(): KorApi {
  return node([]) as KorApi
}
