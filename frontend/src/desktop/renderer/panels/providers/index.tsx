// Sağlayıcılar paneli (§12.3): anahtar/uç nokta yönetimi, sağlık, model kataloğu ve skorlar; ayrıca yerel ağ geçidi
// (kor-proxy), yardımcı bileşenler ve medya işleri. Anahtar ekranı Ayarlar'daki ProvidersSection'ın kendisidir.
import { Server } from 'lucide-react'
import { useId, useState, type KeyboardEvent } from 'react'
import { ProvidersSection } from '../../g9/ProvidersSection.tsx'
import type { KorPanel } from '../registry.ts'
import { CatalogTab } from './CatalogTab.tsx'
import { ComponentsTab } from './ComponentsTab.tsx'
import { HealthTab } from './HealthTab.tsx'
import { MediaTab } from './MediaTab.tsx'
import { ProxyTab } from './ProxyTab.tsx'
import './panel.css'

const TABS = [
  { id: 'keys', label: 'Anahtarlar', Body: ProvidersSection },
  { id: 'health', label: 'Sağlık', Body: HealthTab },
  { id: 'catalog', label: 'Katalog', Body: CatalogTab },
  { id: 'proxy', label: 'Ağ geçidi', Body: ProxyTab },
  { id: 'components', label: 'Bileşenler', Body: ComponentsTab },
  { id: 'media', label: 'Medya', Body: MediaTab },
] as const

type TabId = (typeof TABS)[number]['id']

function ProvidersPanel() {
  const base = useId()
  const [tab, setTab] = useState<TabId>('keys')
  const index = TABS.findIndex((t) => t.id === tab)
  const current = TABS[index] ?? TABS[0]

  const onKey = (e: KeyboardEvent) => {
    const step = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0
    if (step === 0) return
    e.preventDefault()
    const next = TABS[(index + step + TABS.length) % TABS.length]
    if (next) {
      setTab(next.id)
      document.getElementById(`${base}-${next.id}`)?.focus()
    }
  }

  return (
    <div className="kp-panel">
      <div className="g7-tabs kp-tabs" role="tablist" aria-label="Sağlayıcılar bölümleri" onKeyDown={onKey}>
        {TABS.map((t) => (
          <button
            key={t.id}
            id={`${base}-${t.id}`}
            type="button"
            role="tab"
            className="g7-tab"
            aria-selected={t.id === tab}
            aria-controls={`${base}-body`}
            tabIndex={t.id === tab ? 0 : -1}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div id={`${base}-body`} role="tabpanel" aria-labelledby={`${base}-${current.id}`} className="kp-body">
        <current.Body />
      </div>
    </div>
  )
}

export const panel: KorPanel = {
  id: 'providers',
  label: 'Sağlayıcılar',
  icon: Server,
  keywords: 'sağlayıcı provider anahtar key api uç nokta endpoint sağlık health katalog model skor fiyat proxy ağ geçidi bileşen component medya media hugging face indirme',
  component: ProvidersPanel,
}
