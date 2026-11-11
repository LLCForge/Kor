import { Rocket } from 'lucide-react'
import type { KorPanel, KorPanelProps } from '../registry.ts'
import '../graph/panel.css'
import './panel.css'
import { DbSection } from './DbSection.tsx'
import { DeploySection } from './DeploySection.tsx'
import { WatchSection } from './WatchSection.tsx'

function DeployPanel(_: KorPanelProps) {
  return (
    <div className="kd">
      <DeploySection />
      <WatchSection />
      <DbSection />
    </div>
  )
}

export const panel: KorPanel = {
  id: 'deploy',
  label: 'Dağıtım',
  icon: Rocket,
  keywords: 'deploy dağıtım docker compose kubernetes kubectl rollback geri al blue green sağlık health risk migration log izleme watch sentry datadog journald hata incident index migration veritabanı',
  component: DeployPanel,
}
