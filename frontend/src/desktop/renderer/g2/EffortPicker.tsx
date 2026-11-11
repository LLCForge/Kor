import { Brain, ChevronDown } from 'lucide-react'
import { REASONING_EFFORT_LABELS, type ReasoningEffortLevel } from '../../../core/reasoning.ts'
import { usePopover } from '../use-popover.ts'
import { Tooltip } from '../kor-ui/index.ts'

interface EffortPickerProps {
  /** Seçili modelin kabul ettiği düzeyler; boşsa (desteklemiyorsa) seçici gizlenir */
  options: readonly ReasoningEffortLevel[]
  value: ReasoningEffortLevel | null
  onChange: (value: ReasoningEffortLevel | null) => void
}

// cdesktop EFFORT segmenti için kısa etiketler (Low/Med/High/XHigh/Max biçemi).
const SHORT: Readonly<Record<ReasoningEffortLevel, string>> = {
  none: 'Kapalı',
  minimal: 'Min',
  low: 'Düşük',
  medium: 'Orta',
  high: 'Yüksek',
  xhigh: 'Çok Y.',
  max: 'Maks',
}

export function EffortPicker({ options, value, onChange }: EffortPickerProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  if (options.length === 0) return null
  const effective = value && options.includes(value) ? value : null

  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text="Akıl yürütme düzeyi">
        <button
          type="button"
          className="mode-trigger g2-effort"
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={open}
        >
          <Brain size={14} aria-hidden="true" />
          {effective ? REASONING_EFFORT_LABELS[effective] : 'Düşünme'}
          <ChevronDown size={14} className="dim" aria-hidden="true" />
        </button>
      </Tooltip>
      {open && (
        <div className="menu g2-effort-menu pop-in" role="menu" aria-label="Akıl yürütme düzeyi">
          {/* cdesktop: alt satırda "EFFORT" segmenti, seçili turuncu. */}
          <div className="cdx-effort-row">
            <span className="cdx-effort-label">EFFORT</span>
            <div className="cdx-effort-seg" role="radiogroup" aria-label="Akıl yürütme düzeyi">
              <button
                type="button"
                role="radio"
                aria-checked={effective === null}
                className={`cdx-effort-opt${effective === null ? ' on' : ''}`}
                onClick={() => {
                  onChange(null)
                  setOpen(false)
                }}
              >
                Oto
              </button>
              {options.map((id) => (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  aria-checked={id === effective}
                  className={`cdx-effort-opt${id === effective ? ' on' : ''}`}
                  title={REASONING_EFFORT_LABELS[id]}
                  onClick={() => {
                    onChange(id)
                    setOpen(false)
                  }}
                >
                  {SHORT[id]}
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
