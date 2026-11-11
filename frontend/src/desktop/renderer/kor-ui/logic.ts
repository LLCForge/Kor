// kor-ui saf mantığı (DOM'suz, node --test ile sınanır). ADR 0053, spec §23.5.

/** İkon tek başına kullanılırsa tooltip 400 ms gecikmeyle açılır (§23.1). */
export const TOOLTIP_DELAY_MS = 400

/** Segment kontrolünde klavye: ←/→ (ve ↑/↓) dolaşır, Home/End uçlara gider; devre dışı seçenekler atlanır. */
export function segmentedNext<T extends string>(
  options: readonly { value: T; disabled?: boolean }[],
  current: T,
  key: string,
): T | null {
  const enabled = options.filter((o) => !o.disabled)
  if (enabled.length === 0) return null
  const i = enabled.findIndex((o) => o.value === current)
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return enabled[(i + 1 + enabled.length) % enabled.length]!.value
    case 'ArrowLeft':
    case 'ArrowUp':
      return enabled[(i - 1 + enabled.length) % enabled.length]!.value
    case 'Home':
      return enabled[0]!.value
    case 'End':
      return enabled[enabled.length - 1]!.value
    default:
      return null
  }
}

export type ToastKind = 'info' | 'success' | 'warning' | 'error'

export interface ToastItem {
  id: number
  kind: ToastKind
  text: string
  /** ms; 0 = kullanıcı kapatana kadar (hatalar) */
  timeout: number
}

/** Aynı anda en çok bu kadar bildirim görünür; fazlası en eskiyi düşürür. */
export const MAX_TOASTS = 3
const DEFAULT_TIMEOUT: Record<ToastKind, number> = { info: 4000, success: 3000, warning: 6000, error: 0 }

export interface ToastStore {
  push: (text: string, kind?: ToastKind, timeout?: number) => number
  dismiss: (id: number) => void
  list: () => readonly ToastItem[]
  subscribe: (listener: () => void) => () => void
}

export function createToastStore(): ToastStore {
  let items: readonly ToastItem[] = []
  let next = 1
  const listeners = new Set<() => void>()
  const emit = () => {
    for (const l of listeners) l()
  }
  return {
    push(text, kind = 'info', timeout = DEFAULT_TIMEOUT[kind]) {
      const id = next++
      // Uzun hata metni kartı taşırmaz; tam metin kaynağında (log/konsol) durur.
      const clean = text.length > 400 ? `${text.slice(0, 399)}…` : text
      items = [...items, { id, kind, text: clean, timeout }].slice(-MAX_TOASTS)
      emit()
      return id
    },
    dismiss(id) {
      const before = items.length
      items = items.filter((t) => t.id !== id)
      if (items.length !== before) emit()
    },
    list: () => items,
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

/** `cx('a', cond && 'b')`: boş olanlar atlanır. */
export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(' ')
}
