import { X } from 'lucide-react'
import { useEffect, useSyncExternalStore } from 'react'
import { createToastStore, type ToastItem, type ToastKind } from './logic.ts'

/** Uygulama geneli bildirim kuyruğu: `toast('Kaydedildi', 'success')`. */
export const toasts = createToastStore()

export function toast(text: string, kind: ToastKind = 'info', timeout?: number): number {
  return toasts.push(text, kind, timeout)
}

function ToastCard({ item }: { item: ToastItem }) {
  useEffect(() => {
    if (item.timeout <= 0) return
    const t = setTimeout(() => toasts.dismiss(item.id), item.timeout)
    return () => clearTimeout(t)
  }, [item.id, item.timeout])
  return (
    <div className={`kor-toast ${item.kind}`} role={item.kind === 'error' ? 'alert' : 'status'}>
      <span className="kor-toast-text">{item.text}</span>
      <button type="button" className="kor-toast-close" onClick={() => toasts.dismiss(item.id)} aria-label="Bildirimi kapat" title="Kapat">
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  )
}

/** Bildirimlerin çizildiği tek yer; App köküne bir kez konur. */
export function Toaster() {
  const items = useSyncExternalStore(toasts.subscribe, toasts.list)
  return (
    <div className="kor-toaster" aria-live="polite">
      {items.map((t) => (
        <ToastCard key={t.id} item={t} />
      ))}
    </div>
  )
}
