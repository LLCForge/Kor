import type { ReactNode } from 'react'
import { cx } from './logic.ts'

export interface CardProps {
  title?: string
  /** Başlık satırının sağı (eylem butonları) */
  actions?: ReactNode
  /** Yüzey katmanı: surface (varsayılan) ya da raised (§23.1: derinlik gölgeyle değil katmanla). */
  layer?: 'surface' | 'raised'
  children: ReactNode
  className?: string
}

/** Kor kartı: iç boşluk 16, köşe 10, 1 px kenarlık, gölge yok. */
export function Card({ title, actions, layer = 'surface', children, className }: CardProps) {
  return (
    <section className={cx('kor-card', layer, className)} aria-label={title}>
      {(title || actions) && (
        <header className="kor-card-head">
          {title && <h3 className="kor-card-title">{title}</h3>}
          {actions && <span className="kor-card-actions">{actions}</span>}
        </header>
      )}
      {children}
    </section>
  )
}
