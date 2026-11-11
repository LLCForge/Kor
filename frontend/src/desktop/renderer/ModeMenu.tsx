import { AlertTriangle, Check, ChevronDown, FastForward, Hand, List, PenLine } from 'lucide-react'
import type { ComponentType } from 'react'
import { useEffect } from 'react'
import type { PermissionMode } from '../../core/permissions.ts'
import { usePopover } from './use-popover.ts'
import { Tooltip } from './kor-ui/index.ts'

// cdesktop izin modu sırası ve ikonları: Ask / Accept edits / Plan / Auto / Bypass.
export const MODES: ReadonlyArray<{ id: PermissionMode; label: string; desc: string; icon: ComponentType<{ size?: number }>; danger?: boolean }> = [
  { id: 'default', label: 'Manuel', desc: 'Değişiklik yapmadan önce her zaman sor', icon: Hand },
  { id: 'acceptEdits', label: 'Düzenlemeleri kabul et', desc: 'Dosya düzenlemeleri otomatik onaylanır', icon: PenLine },
  { id: 'plan', label: 'Plan', desc: 'Değişiklik yapmadan önce plan oluşturur', icon: List },
  { id: 'auto', label: 'Otomatik', desc: 'Düzenlemeler ve komutlar onaysız; tehlikeli olanlar sorulur', icon: FastForward },
  { id: 'bypass', label: 'İzinleri atla', desc: 'Hiçbir şey sorulmaz, tehlikeli komutlar dahil', icon: AlertTriangle, danger: true },
]

export function nextMode(current: PermissionMode): PermissionMode {
  const i = MODES.findIndex((m) => m.id === current)
  return MODES[(i + 1) % MODES.length]?.id ?? 'default'
}

interface ModeMenuProps {
  value: PermissionMode
  defaultMode: PermissionMode
  onChange: (mode: PermissionMode) => void
  onMakeDefault: (mode: PermissionMode) => void
}

export function ModeMenu({ value, defaultMode, onChange, onMakeDefault }: ModeMenuProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const current = MODES.find((m) => m.id === value) ?? MODES[1]

  // Menü açıkken 1-5 tuşlarıyla seçim.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      const index = Number(e.key) - 1
      const mode = MODES[index]
      if (mode && !e.ctrlKey && !e.altKey && !e.metaKey) {
        e.preventDefault()
        onChange(mode.id)
        setOpen(false)
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, onChange, setOpen])

  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text="İzin modu (Shift+Tab ile değiştir)">
        <button
          type="button"
          className={`mode-trigger mode-${value}`}
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={open}
        >
          {current?.label}
          <ChevronDown size={14} className="dim" aria-hidden="true" />
        </button>
      </Tooltip>

      {open && (
        <div className="menu menu-left pop-in" role="menu" aria-label="İzin modu">
          <p className="menu-heading">Mod</p>
          {MODES.map((m, i) => (
            <button
              key={m.id}
              type="button"
              role="menuitemradio"
              aria-checked={m.id === value}
              className="mode-item"
              onClick={() => {
                onChange(m.id)
                setOpen(false)
              }}
            >
              <span className={`mode-icon${m.danger ? ' danger' : ''}`} aria-hidden="true">
                <m.icon size={16} />
              </span>
              <span className="mode-main">
                <span className={`mode-label${m.danger ? ' danger' : ''}`}>
                  {m.label}
                  {m.id === 'auto' && <span className="badge">Önerilen</span>}
                  {m.id === defaultMode && <span className="badge neutral">Varsayılan</span>}
                </span>
                <span className="mode-desc">{m.desc}</span>
              </span>
              <span className="mode-check">{m.id === value && <Check size={16} aria-hidden="true" />}</span>
              <kbd className="mode-key">{i + 1}</kbd>
            </button>
          ))}
          {value !== defaultMode && (
            <button type="button" className="menu-footer-btn" onClick={() => onMakeDefault(value)}>
              "{current?.label}" modunu varsayılan yap
            </button>
          )}
        </div>
      )}
    </div>
  )
}
