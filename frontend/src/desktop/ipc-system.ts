// Sistem profili ve sağlık (§19.2–19.3), masaüstü denetimi (§19.1), pencere düzenleri (§27.7), ADR 0059:
// `system.*`, `computer.*`, `layout.*`. Rust: frontend/src-tauri/src/ns/system.rs; alan adları
// `kor_health` / `kor_computer` / `kor_platform::desktop_probe` serde çıktısıyla aynıdır.

export interface SystemGpu { name: string; vram_mb: number | null }
export interface SystemDisk { mount: string; name: string; fs: string; total_mb: number; free_mb: number; removable: boolean }
export interface SystemBattery { percent: number; charging: boolean }
export interface SystemDistro { id: string; family: string; name: string; version: string; immutable: boolean }
export interface SystemDisplay { name: string; x: number; y: number; width: number; height: number; scale: number; primary: boolean }
export interface SystemKeyboard { raw: string; lang: string | null; variant: string | null }
export interface SystemProfile {
  collected_at: number
  os: {
    family: string
    name: string
    version: string
    long_name: string
    kernel: string
    arch: string
    hostname: string
    distro: SystemDistro | null
    package_managers: string[]
    desktop: string | null
  }
  cpu: { model: string; cores: number | null; threads: number }
  memory: { total_mb: number; available_mb: number; swap_total_mb: number; swap_used_mb: number }
  gpus: SystemGpu[]
  disks: SystemDisk[]
  battery: SystemBattery | null
  network: Array<{ name: string; addrs: string[] }>
  toolchains: Record<string, string>
  shell: string | null
  terminal: string | null
  displays: SystemDisplay[]
  keyboard: SystemKeyboard | null
}
export interface SystemRefresh { profile: SystemProfile; changed: string[] }

export type HealthLevel = 'warn' | 'critical'
export interface HealthAlert { id: string; kind: string; level: HealthLevel; title: string; body: string; clean: boolean }
export interface CleanCandidate { id: string; kind: string; path: string | null; bytes: number; approx: boolean }
export interface CleanOutcome { id: string; kind: string; path: string | null; freed: number; error: string | null }

export interface ExportSummary { path: string; files: number; bytes: number }

export interface SystemApi {
  profile(): Promise<SystemProfile>
  refresh(): Promise<SystemRefresh>
  health(deep: boolean): Promise<HealthAlert[]>
  cleanSuggest(): Promise<CleanCandidate[]>
  clean(ids: string[]): Promise<CleanOutcome[]>
  mute(kind: string): Promise<string[]>
  exportAll(): Promise<ExportSummary>
  onHealth(listener: (a: HealthAlert) => void): () => void
  onChanged(listener: (r: SystemRefresh) => void): () => void
}

export type ComputerTier = 'blocked' | 'read' | 'click' | 'ask' | 'full'
export type ComputerClass = 'kor' | 'terminal' | 'password_manager' | 'banking' | 'browser' | 'ide' | 'other'
export interface ComputerActivity { active: boolean; tool: string; app: string | null; paused: 'mouse' | 'input' | null; at: number }
export interface ComputerStatus {
  enabled: boolean
  front: { app: string; tier: ComputerTier; class: ComputerClass } | null
  watching: boolean
  paused: 'mouse' | 'input' | null
  activity: ComputerActivity
  apps: Array<{ app: string; tier: ComputerTier; source: 'config' | 'default'; class: ComputerClass }>
  classes: Array<{ class: ComputerClass; tier: ComputerTier }>
}
export interface ComputerApi {
  status(): Promise<ComputerStatus>
  setEnabled(on: boolean): Promise<ComputerStatus>
  setTier(app: string, tier: ComputerTier): Promise<ComputerStatus>
  resetTier(app: string): Promise<ComputerStatus>
  stop(): Promise<ComputerStatus>
  onActivity(listener: (a: ComputerActivity) => void): () => void
}

export interface LayoutMonitor { name: string; x: number; y: number; width: number; height: number; primary: boolean }
export interface LayoutSaved { process: string; title: string; monitor: string; rx: number; ry: number; rw: number; rh: number; state: string }
export interface WindowLayout { name: string; saved_at: number; monitors: LayoutMonitor[]; windows: LayoutSaved[] }
export interface LayoutMove { hwnd: number; process: string; title: string; x: number; y: number; w: number; h: number; state: string }
export interface LayoutPlan { moves: LayoutMove[]; missing: string[] }
export interface LayoutApi {
  list(): Promise<{ layouts: WindowLayout[]; linked: string | null }>
  linked(): Promise<string | null>
  save(name: string, linkProject: boolean): Promise<WindowLayout>
  restore(name: string): Promise<LayoutPlan>
  delete(name: string): Promise<null>
  onRestored(listener: (e: { name: string; plan: LayoutPlan }) => void): () => void
}
