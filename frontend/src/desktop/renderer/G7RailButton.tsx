import { Settings } from 'lucide-react'
import { openG7Dialog } from './g7-bus.ts'

/** Kenar çubuğu alt düğmesi; Sidebar'daki RailButton ile aynı görünüm. */
export function SettingsRailButton({ collapsed }: { collapsed: boolean }) {
  return (
    <button
      type="button"
      className="rail-btn"
      onClick={() => openG7Dialog({ kind: 'settings' })}
      title={collapsed ? 'Ayarlar' : undefined}
      aria-label={collapsed ? 'Ayarlar' : undefined}
    >
      <Settings size={16} aria-hidden="true" />
      <span className="rail-btn-label">Ayarlar</span>
    </button>
  )
}
