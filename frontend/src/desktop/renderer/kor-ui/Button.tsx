import type { ButtonHTMLAttributes, ReactNode } from 'react'
import { cx } from './logic.ts'

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger'

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
  size?: 'small' | 'default'
  /** Lucide ikonu (aria-hidden). Yalnız ikonlu butonda `aria-label` ve `title` verin. */
  icon?: ReactNode
  loading?: boolean
  children?: ReactNode
}

/**
 * Kor butonu: mevcut `.btn` stillerinin ince sarmalayıcısı (§23.5). Ember yalnızca `primary`'de (§23.1).
 * `loading` iken tıklanamaz ve `aria-busy` taşır; ayrı spinner çizilmez (aynı anda tek spinner kuralı).
 */
export function Button({ variant = 'secondary', size = 'default', icon, loading = false, className, children, disabled, type = 'button', ...rest }: ButtonProps) {
  return (
    <button
      type={type}
      className={cx('btn', 'kor-btn', variant !== 'secondary' && variant, size === 'small' && 'small', !children && 'kor-btn-icon', loading && 'loading', className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {icon}
      {children}
    </button>
  )
}
