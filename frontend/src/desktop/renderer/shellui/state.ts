// Kabuk varlığı görünümlerinin saf yardımcıları (ADR 0051): zil, onay kuyruğu, PiP, tepsi açılır penceresi.
// React içe aktarmaz; node testleri doğrudan kullanır.
import type { Eta, KorNotification, NotificationCategory, QueueItem, Risk, RunningSession, ShellState } from '../../ipc-shellui.ts'

/** `1:05`, `12:40`, `1 sa 02 dk` */
export function elapsed(since: number, now: number): string {
  const s = Math.max(0, Math.floor((now - since) / 1000))
  if (s >= 3600) return `${Math.floor(s / 3600)} sa ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')} dk`
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/** `$0.84`, tahminse `≈$0.84` (§5.2). */
export function usd(value: number, estimated: boolean): string {
  return `${estimated ? '≈' : ''}$${value.toFixed(2)}`
}

function clock(ms: number): string {
  const d = new Date(ms)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

/** Yöntem açıklaması: tahminin nereden geldiği. */
export function etaMethod(e: Eta): string {
  const where = e.scope === 'project' ? `bu projedeki ${e.samples} turun` : `son ${e.samples} turun`
  return `${where} p${e.percentile} değeri`
}

/** `≈ 14:32'de biter` / `Her zamankinden uzun sürüyor`; tahmin yoksa null. */
export function etaLabel(e: Eta | null): string | null {
  if (!e) return null
  if (e.overdue || e.finishMs === null) return 'Her zamankinden uzun sürüyor'
  return `≈ ${clock(e.finishMs)} civarı biter`
}

/** Geri sayım: `≈ 3 dk` / `≈ 40 sn`. */
export function countdown(e: Eta | null, now: number): string | null {
  if (!e || e.overdue || e.finishMs === null) return null
  const left = Math.max(0, Math.round((e.finishMs - now) / 1000))
  return left >= 60 ? `≈ ${Math.round(left / 60)} dk` : `≈ ${left} sn`
}

export type QueueAction = 'approve' | 'deny' | 'next' | 'prev' | 'details' | 'approveSimilar' | 'close'

/** §28.5 klavye: Enter/→/Y onayla, ←/N reddet, ↓/J sonraki (ertele), ↑/K önceki, D ayrıntı, A benzerleri onayla, Esc/Q kapat — `kor approvals` ile aynı. */
export function queueAction(key: string): QueueAction | null {
  switch (key) {
    case 'Enter':
    case 'ArrowRight':
    case 'y':
    case 'Y':
      return 'approve'
    case 'ArrowLeft':
    case 'n':
    case 'N':
      return 'deny'
    case 'ArrowDown':
    case 'j':
    case 'J':
      return 'next'
    case 'ArrowUp':
    case 'k':
    case 'K':
      return 'prev'
    case 'd':
    case 'D':
      return 'details'
    case 'a':
    case 'A':
      return 'approveSimilar'
    case 'Escape':
    case 'q':
    case 'Q':
      return 'close'
    default:
      return null
  }
}

/** Kuyrukta gezinme: seçili kartın yeni dizini (sarmalı). */
export function move(index: number, length: number, action: 'next' | 'prev'): number {
  if (length === 0) return 0
  return action === 'next' ? (index + 1) % length : (index - 1 + length) % length
}

/** Aynı gruptaki onayların kimlikleri (en az iki ise "hepsini onayla"). */
export function similarIds(items: readonly QueueItem[], item: QueueItem): string[] {
  return items.filter((i) => i.group === item.group).map((i) => i.id)
}

export const RISK_LABEL: Readonly<Record<Risk, string>> = { low: 'Düşük risk', medium: 'Orta risk', high: 'Yüksek risk' }

export const SOURCE_LABEL: Readonly<Record<QueueItem['source'], string>> = { desktop: 'Sohbet', daemon: 'Daemon', remote: 'Uzak görev' }

export const CATEGORY_LABEL: Readonly<Record<NotificationCategory, string>> = {
  approval: 'Onay',
  question: 'Soru',
  done: 'Bitti',
  error: 'Hata',
  budget: 'Bütçe',
  update: 'Güncelleme',
  health: 'Sağlık',
  component: 'Bileşen',
  quota: 'Kota',
}

export type NotificationFilter = 'all' | 'unread' | NotificationCategory

export function filterNotifications(items: readonly KorNotification[], filter: NotificationFilter): KorNotification[] {
  if (filter === 'all') return [...items]
  if (filter === 'unread') return items.filter((n) => !n.read)
  return items.filter((n) => n.category === filter)
}

/** Listede bulunan kategoriler (filtre düğmeleri yalnızca bunlar için). */
export function presentCategories(items: readonly KorNotification[]): NotificationCategory[] {
  const order = Object.keys(CATEGORY_LABEL) as NotificationCategory[]
  const seen = new Set(items.map((n) => n.category))
  return order.filter((c) => seen.has(c))
}

export type TrayStatus = 'idle' | 'working' | 'waiting'

export function trayStatus(s: ShellState): TrayStatus {
  if (s.overview.approvals.length > 0) return 'waiting'
  if (s.overview.sessions.length > 0 || s.overview.daemon.jobs.some((j) => j.status === 'running')) return 'working'
  return 'idle'
}

/** PiP'in gösterdiği görev: onay bekleyen önce, sonra en son başlayan. */
export function primarySession(sessions: readonly RunningSession[]): RunningSession | null {
  if (sessions.length === 0) return null
  const waiting = sessions.find((s) => s.waiting)
  if (waiting) return waiting
  return [...sessions].sort((a, b) => b.since - a.since)[0] ?? null
}

/** İlerleme yüzdesi (yapılacaklar listesinden); yoksa null (belirsiz çizgi). */
export function percent(s: RunningSession): number | null {
  if (!s.progress || s.progress[1] === 0) return null
  return Math.round((s.progress[0] / s.progress[1]) * 100)
}

/** Görevin bekleyen ilk onayı (PiP'in Onayla/Reddet düğmeleri). */
export function approvalFor(items: readonly QueueItem[], sessionId: string): QueueItem | null {
  return items.find((i) => i.source === 'desktop' && i.owner === sessionId) ?? null
}

export function updatePillLabel(version: string): string {
  return `Güncelle · v${version.replace(/^v/, '')}`
}

/** Son kullanılan klasörün kısa adı. */
export function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}

/** Gizlilik saatleri girdisi: boş ya da `SS-SS` (ör. 22-08). */
export function validQuietHours(v: string): boolean {
  const t = v.trim()
  if (t === '') return true
  const m = /^(\d{1,2})\s*-\s*(\d{1,2})$/.exec(t)
  if (!m) return false
  const a = Number(m[1])
  const b = Number(m[2])
  return a < 24 && b < 24 && a !== b
}
