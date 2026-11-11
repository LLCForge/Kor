// Görünüm kataloğu: sağ panelin yerleşik görünümleri + panels/registry.ts, ve bunların arayüzde nerede durduğu
// (panel başlığı sekmeleri, kenar çubuğu "Araçlar" grupları, Ayarlar bölümleri). Tüm görünümler komut paletinde de kalır.
import { Activity, Bot, Columns2, Compass, Eye, FolderKanban, GitBranch, Globe, ScrollText, SquareTerminal, Stethoscope, StickyNote } from 'lucide-react'
import type { ComponentType } from 'react'
import { findPanel } from './panels/registry.ts'
import { OPEN_TARGET_EVENT } from './shellui/bridge.ts'

export interface ViewInfo {
  id: string
  label: string
  icon: ComponentType<{ size?: number }>
}

export const BUILTIN_VIEWS: readonly ViewInfo[] = [
  { id: 'changes', label: 'Değişiklikler', icon: Columns2 },
  { id: 'preview', label: 'Önizleme', icon: Eye },
  { id: 'devpreview', label: 'Web önizleme', icon: Globe },
  { id: 'browser', label: 'Tarayıcı', icon: Compass },
  { id: 'git', label: 'Git', icon: GitBranch },
  { id: 'terminal', label: 'Terminal', icon: SquareTerminal },
  { id: 'notes', label: 'Notlar', icon: StickyNote },
  { id: 'logs', label: 'Günlükler', icon: ScrollText },
  { id: 'diagnostics', label: 'Tanılama', icon: Stethoscope },
]

/** Kimliği bilinmeyen (ör. kaldırılmış görünüm) için undefined: kayıtlı tercihler sessizce elenir. */
export function viewInfo(id: string): ViewInfo | undefined {
  return BUILTIN_VIEWS.find((v) => v.id === id) ?? findPanel(id)
}

/** Panel başlığında her zaman görünen sekmeler. */
export const HEADER_TABS: readonly string[] = ['changes', 'preview', 'git', 'terminal', 'notes']

export interface ViewGroup {
  id: string
  label: string
  icon: ComponentType<{ size?: number }>
  views: readonly string[]
}

/** Kenar çubuğu "Araçlar" başlatıcısı. */
export const SIDEBAR_VIEW_GROUPS: readonly ViewGroup[] = [
  { id: 'project', label: 'Proje', icon: FolderKanban, views: ['graph', 'packages', 'analysis', 'diagrams', 'knowledge', 'docs', 'memory', 'context'] },
  { id: 'agents', label: 'Ajanlar', icon: Bot, views: ['tasks', 'plans', 'sessionTree', 'forks', 'replay', 'rules'] },
  { id: 'web', label: 'Web ve medya', icon: Globe, views: ['web', 'media', 'visual', 'browser', 'devpreview'] },
  { id: 'ops', label: 'Operasyon', icon: Activity, views: ['deploy', 'triggers', 'remote', 'peers', 'logs', 'diagnostics', 'perf', 'cost'] },
]

/** Sağ paneli açar ve görünümü seçer (App, OPEN_TARGET_EVENT → openPanel). sessionId verilirse önce o sohbet açılır. */
export function openView(id: string, sessionId: string | null = null): void {
  window.dispatchEvent(new CustomEvent(OPEN_TARGET_EVENT, { detail: { sessionId, panel: id } }))
}

/** Komut paletini "Görünüm" sayfasında açar. */
export function openAllViews(): void {
  window.dispatchEvent(new CustomEvent('osint:palette', { detail: 'views' }))
}
