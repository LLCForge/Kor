// [cdx] Ajan yapılandırma profilleri (cdesktop "Default ⌄" pill): tek tıkla izin modu + model + akıl yürütme düzeyi.
// Yerleşik profiller her zaman vardır; kullanıcı profilleri Ayarlar > Genel > Profiller'den gelir.
import { Check, ChevronDown, Settings2, UserCircle2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { PermissionMode } from '../../core/permissions.ts'
import type { AgentProfile } from '../g7-types.ts'
import { api } from './api.ts'
import { openG7Dialog } from './g7-bus.ts'
import { setG2Prefs } from './g2/prefs.ts'
import { MODES } from './ModeMenu.tsx'
import { usePopover } from './use-popover.ts'

const KEY = 'kor.cdx.profile'

export const BUILTIN_PROFILES: ReadonlyArray<AgentProfile & { builtin: true; desc: string }> = [
  { name: 'Varsayılan', builtin: true, desc: 'Ayarlardaki varsayılan mod; model ve düzey değişmez' },
  { name: 'Onaylı', mode: 'default', builtin: true, desc: 'Her değişiklik önce sorulur' },
  { name: 'Plan', mode: 'plan', builtin: true, desc: 'Önce plan, sonra uygulama' },
  { name: 'Otomatik', mode: 'auto', effort: 'high', builtin: true, desc: 'Onaysız çalışır, yüksek akıl yürütme' },
]

function loadActive(): string {
  try {
    return localStorage.getItem(KEY) ?? 'Varsayılan'
  } catch {
    return 'Varsayılan'
  }
}

interface Props {
  permissionMode: PermissionMode
  defaultMode: PermissionMode
  model: string
  onPermissionModeChange: (mode: PermissionMode) => void
  onModelChange: (id: string) => void
}

export function describeProfile(p: AgentProfile, defaultMode: PermissionMode): string {
  const parts: string[] = []
  const mode = MODES.find((m) => m.id === (p.mode ?? defaultMode))
  if (p.mode && mode) parts.push(mode.label)
  if (p.model) parts.push(p.model)
  if (p.effort) parts.push(`akıl yürütme: ${p.effort}`)
  return parts.join(' · ')
}

export function ProfileMenu({ permissionMode, defaultMode, model, onPermissionModeChange, onModelChange }: Props) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const { data: settings } = useSWR('g7:settings', () => api.g7GetSettings(), { revalidateOnFocus: false })
  const [active, setActive] = useState(loadActive)
  const custom = settings?.profiles ?? []
  const all: ReadonlyArray<AgentProfile & { builtin?: true; desc?: string }> = [...BUILTIN_PROFILES, ...custom]
  const current = all.find((p) => p.name === active) ?? BUILTIN_PROFILES[0]!
  // Profil seçili olduğu hâlde kullanıcı modu elle değiştirirse pill "Varsayılan"a döner (cdesktop: profile dirty → Default).
  useEffect(() => {
    if (current.mode && current.mode !== permissionMode) setActive('Varsayılan')
    if (current.model && current.model !== model) setActive('Varsayılan')
  }, [permissionMode, model]) // eslint-disable-line react-hooks/exhaustive-deps

  const apply = (p: AgentProfile) => {
    setActive(p.name)
    try {
      localStorage.setItem(KEY, p.name)
    } catch {
      /* yalnızca kolaylık */
    }
    if (p.mode) onPermissionModeChange(p.mode)
    else if (p.name === 'Varsayılan') onPermissionModeChange(defaultMode)
    if (p.model) onModelChange(p.model)
    if (p.effort) setG2Prefs({ effort: p.effort })
    setOpen(false)
  }

  return (
    <div className="popover-anchor" ref={ref}>
      <button type="button" className={`mode-trigger cdx-profile-trigger${active !== 'Varsayılan' ? ' active' : ''}`} onClick={() => setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={open} title="Ajan profili">
        <UserCircle2 size={13} aria-hidden="true" />
        {current.name}
        <ChevronDown size={14} className="dim" aria-hidden="true" />
      </button>
      {open && (
        <div className="menu menu-left pop-in cdx-profile-menu" role="menu" aria-label="Ajan profili">
          <p className="menu-heading">Profil</p>
          {all.map((p) => (
            <button key={p.name} type="button" role="menuitemradio" aria-checked={p.name === active} className="mode-item" onClick={() => apply(p)}>
              <span className="mode-main">
                <span className="mode-label">
                  {p.name}
                  {!p.builtin && <span className="badge neutral">Özel</span>}
                </span>
                <span className="mode-desc">{p.desc ?? describeProfile(p, defaultMode) ?? ''}</span>
              </span>
              <span className="mode-check">{p.name === active && <Check size={16} aria-hidden="true" />}</span>
            </button>
          ))}
          <button type="button" className="menu-footer-btn" onClick={() => { setOpen(false); openG7Dialog({ kind: 'settings', section: 'general' }) }}>
            <Settings2 size={13} aria-hidden="true" /> Profilleri düzenle…
          </button>
        </div>
      )}
    </div>
  )
}
