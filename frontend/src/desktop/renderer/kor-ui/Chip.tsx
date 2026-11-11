import type { ReactNode } from 'react'
import { X } from 'lucide-react'
import { cx } from './logic.ts'

export interface ChipProps {
  label: string
  icon?: ReactNode
  /** Seçili/etkin çip ember kenarlık alır. */
  selected?: boolean
  onClick?: () => void
  /** Kaldırma düğmesi (ör. ek dosya çipi). */
  onRemove?: () => void
  title?: string
  disabled?: boolean
  /** §16.3: çipin çağırdığı backend komutu */
  cmd?: string
}

/** Kor çipi: bağlam, filtre ve hızlı eylem çipleri (§23.3). Tıklanabilirse buton, değilse etiket. */
export function Chip({ label, icon, selected = false, onClick, onRemove, title, disabled, cmd }: ChipProps) {
  const body = (
    <>
      {icon}
      <span className="kor-chip-label">{label}</span>
    </>
  )
  return (
    <span className={cx('kor-chip', selected && 'selected', disabled && 'disabled')} title={title}>
      {onClick ? (
        <button type="button" className="kor-chip-main" onClick={onClick} disabled={disabled} aria-pressed={selected} data-kor-cmd={cmd}>
          {body}
        </button>
      ) : (
        <span className="kor-chip-main">{body}</span>
      )}
      {onRemove && (
        <button type="button" className="kor-chip-remove" onClick={onRemove} disabled={disabled} aria-label={`${label}: kaldır`} title="Kaldır">
          <X size={12} aria-hidden="true" />
        </button>
      )}
    </span>
  )
}
