import { BellRing } from 'lucide-react'
import { useState } from 'react'
import type { KorPanel } from '../registry.ts'
import { ApprovalQueue } from '../../shellui/ApprovalQueue.tsx'
import { ShellSettings } from '../../shellui/ShellSettings.tsx'
import '../../shellui/shellui.css'

function ShellPanel() {
  const [tab, setTab] = useState<'queue' | 'settings'>('queue')
  return (
    <section className="ksh-panel" aria-label="Onaylar ve bildirimler">
      <div className="ksh-filters" role="tablist" aria-label="Görünüm">
        <button type="button" role="tab" aria-selected={tab === 'queue'} className={`ksh-chip-btn${tab === 'queue' ? ' active' : ''}`} onClick={() => setTab('queue')}>
          Onay kuyruğu
        </button>
        <button type="button" role="tab" aria-selected={tab === 'settings'} className={`ksh-chip-btn${tab === 'settings' ? ' active' : ''}`} onClick={() => setTab('settings')}>
          Bildirim ayarları
        </button>
      </div>
      {tab === 'queue' ? <ApprovalQueue /> : <ShellSettings />}
    </section>
  )
}

export const panel: KorPanel = {
  id: 'shellui',
  label: 'Onaylar ve bildirimler',
  icon: BellRing,
  keywords: 'onay kuyruğu approval queue bildirim notification tepsi tray pip durum penceresi beni çağır sessiz saat boşta eta',
  component: ShellPanel,
}
