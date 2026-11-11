import { Zap } from 'lucide-react'
import type { ModelInfo } from '../../ipc.ts'
import { fastVariant } from './composer-utils.ts'

interface FastToggleProps {
  model: string
  models: readonly ModelInfo[] | undefined
  onChange: (id: string) => void
}

/** Seçili modelin "-fast" varyantı varsa ona geçirir; yoksa devre dışı ve açıklamalı. */
export function FastToggle({ model, models, onChange }: FastToggleProps) {
  if (!models) return null
  const { on, target } = fastVariant(model, models.map((m) => m.id))
  const title = on
    ? target
      ? `Hızlı mod açık; kapatınca ${target} modeline dönülür`
      : 'Hızlı mod açık; temel model listede yok'
    : target
      ? `Hızlı varyanta geç (${target})`
      : 'Bu modelin hızlı (-fast) varyantı yok'

  return (
    <button
      type="button"
      className={`action-btn g2-fast${on ? ' active' : ''}`}
      onClick={() => target && onChange(target)}
      disabled={!target}
      aria-pressed={on}
      aria-label="Hızlı mod"
      title={title}
    >
      <Zap size={14} aria-hidden="true" />
      <span className="g2-fast-label">Hızlı</span>
    </button>
  )
}
