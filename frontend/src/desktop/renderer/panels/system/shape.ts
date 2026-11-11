// Sistem görünümü için saf yardımcılar (ADR 0059); node --test ile sınanır.
import type { ComputerActivity, ComputerClass, ComputerTier, HealthAlert, SystemDisk } from '../../../ipc-system.ts'

export const TIERS: ReadonlyArray<ComputerTier> = ['blocked', 'read', 'click', 'ask', 'full']

export const TIER_LABEL: Record<ComputerTier, string> = {
  blocked: 'Yasak',
  read: 'Yalnızca okuma',
  click: 'Tıklama',
  ask: 'Her adımda onay',
  full: 'Serbest',
}

export const CLASS_LABEL: Record<ComputerClass, string> = {
  kor: "Kor'un kendisi",
  terminal: 'Terminaller',
  password_manager: 'Parola yöneticileri',
  banking: 'Banka ve cüzdan uygulamaları',
  browser: 'Tarayıcılar',
  ide: "IDE'ler ve kod düzenleyiciler",
  other: 'Diğer uygulamalar',
}

/** 1024 tabanlı boyut: `18.0 GB`, `640 MB`. */
export function bytes(n: number): string {
  const units = ['B', 'kB', 'MB', 'GB', 'TB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return i === 0 || v >= 100 ? `${Math.round(v)} ${units[i]}` : `${v.toFixed(1)} ${units[i]}`
}

export const mb = (n: number): string => bytes(n * 1024 * 1024)

export function diskUsed(d: SystemDisk): number {
  if (d.total_mb <= 0) return 0
  return Math.min(100, Math.floor(((d.total_mb - Math.min(d.free_mb, d.total_mb)) * 100) / d.total_mb))
}

/** Sağ alttaki kart yığını: aynı uyarı bir kez, en yeni üstte, en çok `max` kart. */
export function pushCard(cards: HealthAlert[], a: HealthAlert, max = 3): HealthAlert[] {
  return [a, ...cards.filter((c) => c.id !== a.id)].slice(0, max)
}

/** Gösterge: son eylemden bu yana `ms` geçmediyse "kontrol ediyor". */
export function controlling(a: ComputerActivity | null, now: number, ms = 5000): boolean {
  return a !== null && a.active && a.paused === null && now - a.at <= ms
}

/** Uygulama adı yapılandırma anahtarı olabilir mi (nokta yok, 1–64 karakter). */
export function validApp(name: string): boolean {
  const n = name.trim()
  return n.length > 0 && n.length <= 64 && /^[\p{L}\p{N} _+-]+$/u.test(n)
}

/** Düzen adı: harf, rakam, - _ . ; noktayla başlamaz. */
export function validLayout(name: string): boolean {
  return name.length > 0 && name.length <= 64 && !name.startsWith('.') && /^[\p{L}\p{N}._-]+$/u.test(name)
}
