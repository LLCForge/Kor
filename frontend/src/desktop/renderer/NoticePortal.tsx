import type { ReactNode } from 'react'
import { createPortal } from 'react-dom'

/** Uyarı bantları tek yerde, başlığın altında ve akışın içinde durur (`.kor-notices`): sohbeti örtmez,
 *  birbirinin üstüne binmez. Ev sahibi henüz yoksa bantlar eski yerinde (yüzen) çizilir. */
export function NoticePortal({ host, children }: { host: HTMLElement | null; children: ReactNode }) {
  return host ? createPortal(children, host) : <>{children}</>
}
