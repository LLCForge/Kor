// Niyet kararlarının metinleri (§30). Saf işlevler: node testleri burada.
import type { IntentDecision, IntentPlan } from '../../ipc-intent.ts'

/** `≈12 sn, ≈$0.01` (bilinmeyen kısım yazılmaz) */
export function extra(p: IntentPlan): string {
  const parts: string[] = []
  if (p.eta_s != null) parts.push(`≈${p.eta_s} sn`)
  if (p.usd != null) parts.push(`≈$${p.usd.toFixed(2)}`)
  return parts.join(', ')
}

export function viaLabel(p: IntentPlan): string {
  switch (p.via) {
    case 'keyword':
      return 'anahtar kelimeler'
    case 'learned':
      return 'sizin cümleniz'
    case 'embedding':
      return 'yerel gömme'
    case 'model':
      return 'model'
  }
}

/** Kararın kartta gösterilecek başlığı. */
export function headline(d: IntentDecision): string {
  switch (d.kind) {
    case 'agent':
      return 'Ajana gönderiliyor'
    case 'run':
      return 'Salt okunur komut çalıştırılıyor'
    case 'confirm':
      return 'Bu komut çalıştırılsın mı?'
    case 'missing':
      return 'Komut için eksik bilgi var'
    case 'choose':
      return 'Hangisini kastettiniz?'
    case 'pipeline':
      return `${d.steps.length} adım sırayla çalışacak`
    case 'schedule':
      return 'Zamanlanmış görev oluşturulsun mu?'
  }
}

/** `her 10080 dk` → `her hafta` gibi okunur aralık. */
export function interval(minutes: number): string {
  if (minutes % 10080 === 0) return minutes === 10080 ? 'her hafta' : `her ${minutes / 10080} haftada bir`
  if (minutes % 1440 === 0) return minutes === 1440 ? 'her gün' : `her ${minutes / 1440} günde bir`
  if (minutes % 60 === 0) return minutes === 60 ? 'her saat' : `her ${minutes / 60} saatte bir`
  return `her ${minutes} dakikada bir`
}

/** Eksik değerler yazılınca metin yeniden yönlendirilir: değerler cümlenin sonuna eklenir. */
export function refill(text: string, values: Record<string, string>): string {
  const filled = Object.values(values).map((v) => v.trim()).filter((v) => v !== '')
  return [text.trim(), ...filled].join(' ')
}

/** Karar kullanıcının bir şey yapmasını gerektiriyor mu? (`run` ve `agent` hemen işlenir) */
export function needsUser(d: IntentDecision): boolean {
  return d.kind !== 'agent' && d.kind !== 'run'
}
