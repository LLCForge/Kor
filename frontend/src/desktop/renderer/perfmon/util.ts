// §32.3 (ADR 0062): bindirme ve ölçüm koşusunun saf yardımcıları (node testleri bunları doğrudan çalıştırır).
import type { ChatItem } from '../chat-state.ts'
import type { PerfmonStat } from '../../ipc-perfmon.ts'
import { percentile, type SlowEntry } from './stats.ts'

/** §32.2: uzun sohbette yalnızca son bu kadar bölüm çizilir; yukarı kaydırınca bir o kadar daha eklenir. */
export const THREAD_WINDOW = 150

/** Pencerede görünen bölümler: son `shown` tanesi (gerisi DOM'a girmez). */
export function windowed<T>(segments: readonly T[], shown: number): { hidden: number; visible: readonly T[] } {
  const hidden = Math.max(0, segments.length - shown)
  return { hidden, visible: hidden > 0 ? segments.slice(hidden) : segments }
}

/** `Ctrl+Shift+P` / `Cmd+Shift+P`. */
export function isToggle(e: Pick<KeyboardEvent, 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey' | 'key'>): boolean {
  return (e.ctrlKey || e.metaKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'p'
}

/** perf.log'a saniyede en çok `perSecond` satır (yavaşlık anında köprüyü doldurmamak için). */
export function throttled(send: (e: SlowEntry) => void, perSecond = 4, now: () => number = () => performance.now()): (e: SlowEntry) => void {
  let windowStart = Number.NEGATIVE_INFINITY
  let sent = 0
  return (e) => {
    const t = now()
    if (t - windowStart >= 1000) {
      windowStart = t
      sent = 0
    }
    if (sent < perSecond) {
      sent++
      send(e)
    }
  }
}

export function stat(samples: readonly number[]): PerfmonStat {
  return { p50: percentile(samples, 0.5), p95: percentile(samples, 0.95), n: samples.length }
}

/** `n` mesajlık sentetik bir sohbet (kullanıcı/yanıt sırayla, yanıtlarda kısa Markdown). */
export function syntheticThread(n: number): ChatItem[] {
  const items: ChatItem[] = []
  for (let i = 0; i < n; i++) {
    items.push(
      i % 2 === 0
        ? { kind: 'user', id: `u${i}`, text: `Soru ${i}: bu dosyadaki hatayı düzelt`, attachments: [] }
        : { kind: 'assistant', id: `a${i}`, text: `Yanıt ${i}: \`src/app.rs\` içinde **dönüş değeri** kontrol edilmiyordu; düzeltildi.\n\n- adım 1\n- adım 2` },
    )
  }
  return items
}
