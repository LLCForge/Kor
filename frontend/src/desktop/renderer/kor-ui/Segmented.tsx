import type { KeyboardEvent, ReactNode } from 'react'
import { cx, segmentedNext } from './logic.ts'

export interface SegmentOption<T extends string> {
  value: T
  label: string
  icon?: ReactNode
  disabled?: boolean
}

export interface SegmentedProps<T extends string> {
  label: string
  value: T
  options: readonly SegmentOption<T>[]
  onChange: (value: T) => void
}

/** Segment kontrolü (§23.3 "Yaz / Önizle"): `radiogroup`, ok tuşlarıyla dolaşılır, seçili segment ember çizgili. */
export function Segmented<T extends string>({ label, value, options, onChange }: SegmentedProps<T>) {
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const next = segmentedNext(options, value, e.key)
    if (next === null) return
    e.preventDefault()
    onChange(next)
    e.currentTarget.querySelector<HTMLButtonElement>(`[data-value="${CSS.escape(next)}"]`)?.focus()
  }
  return (
    <div className="kor-segmented" role="radiogroup" aria-label={label} onKeyDown={onKey}>
      {options.map((o) => {
        const checked = o.value === value
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={checked}
            data-value={o.value}
            tabIndex={checked ? 0 : -1}
            disabled={o.disabled}
            className={cx('kor-segment', checked && 'checked')}
            onClick={() => onChange(o.value)}
          >
            {o.icon}
            <span>{o.label}</span>
          </button>
        )
      })}
    </div>
  )
}
