import { Gauge } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import '../graph/panel.css'
import '../security/panel.css'
import './panel.css'
import { PerfPanel } from './PerfPanel.tsx'

export const panel: KorPanel = {
  id: 'perf',
  label: 'Performans',
  icon: Gauge,
  keywords: 'perf performans profil profiler flamegraph sıcak fonksiyon hot cpu bellek memory allocation samply pprof py-spy cpuprofile yük load benchmark optimize iyileştir',
  component: PerfPanel,
}
