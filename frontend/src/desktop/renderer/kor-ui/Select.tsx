import { useId } from 'react'

export interface SelectOption<T extends string> {
  value: T
  label: string
  disabled?: boolean
}

export interface SelectProps<T extends string> {
  label: string
  /** Etiket görsel olarak gizlenir ama ekran okuyucuya okunur. */
  hideLabel?: boolean
  value: T
  options: readonly SelectOption<T>[]
  onChange: (value: T) => void
  disabled?: boolean
  hint?: string
}

/** Kor seçimi: yerel `<select>` (klavye, ekran okuyucu ve OS seçicisi hazır), token'larla biçimlenmiş. */
export function Select<T extends string>({ label, hideLabel = false, value, options, onChange, disabled, hint }: SelectProps<T>) {
  const id = useId()
  const hintId = `${id}-hint`
  return (
    <span className="kor-select">
      <label htmlFor={id} className={hideLabel ? 'sr-only' : 'kor-select-label'}>
        {label}
      </label>
      <select
        id={id}
        value={value}
        disabled={disabled}
        aria-describedby={hint ? hintId : undefined}
        onChange={(e) => {
          const next = options.find((o) => o.value === e.target.value)
          if (next && !next.disabled) onChange(next.value)
        }}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </option>
        ))}
      </select>
      {hint && (
        <span id={hintId} className="kor-select-hint">
          {hint}
        </span>
      )}
    </span>
  )
}
