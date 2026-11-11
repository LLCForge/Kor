// [g3] Mesaj eylem çubuğu (üzerine gelince görünür): kopyala, ek düğmeler ve zaman damgası.
// Diğer gruplar (ör. g2 düzenle/yeniden üret) düğmelerini children olarak ekler.
import type { ReactNode } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { uiLocale } from './ui-locale.ts'

const timeFormat = new Intl.DateTimeFormat(uiLocale(), { hour: '2-digit', minute: '2-digit' })
const fullFormat = new Intl.DateTimeFormat(uiLocale(), { dateStyle: 'long', timeStyle: 'medium' })

interface MessageActionsProps {
  getText: () => string
  /** Mesajın oluşturulma anı (epoch ms); kayıttan açılan sohbetlerde yoktur */
  at: number | undefined
  children?: ReactNode
}

export function MessageActions({ getText, at, children }: MessageActionsProps) {
  return (
    <div className="action-bar">
      <CopyButton getText={getText} />
      {children}
      {at !== undefined && (
        <time className="g3-time" dateTime={new Date(at).toISOString()} title={fullFormat.format(at)}>
          {timeFormat.format(at)}
        </time>
      )}
    </div>
  )
}
