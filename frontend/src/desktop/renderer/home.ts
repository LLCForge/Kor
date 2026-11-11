// Karşılama ekranının saf mantığı (spec §23.3, §23.4, ADR 0053): günün saatine göre selam, bağlama duyarlı hızlı
// eylem çipleri, sürüm rozeti metni. DOM'suz; node --test: home.test.ts.

/** "İyi akşamlar" + açık projenin adı (yoksa yalnız selam). */
/** Günün saatine göre selam ("İyi geceler"); arayüz onu ayrı metin olarak çizer ki çevrilsin. */
export function greetingWord(hour: number): string {
  return hour >= 5 && hour < 12 ? 'Günaydın' : hour >= 12 && hour < 18 ? 'İyi günler' : hour >= 18 && hour < 23 ? 'İyi akşamlar' : 'İyi geceler'
}

export function greeting(hour: number, projectName: string | null): string {
  const g = greetingWord(hour)
  return projectName ? `${g} — ${projectName}` : g
}

/** Klasör yolunun son bileşeni (Windows ve POSIX ayırıcıları). */
export function projectName(root: string | null): string | null {
  if (!root) return null
  const parts = root.replace(/[\\/]+$/, '').split(/[\\/]/)
  return parts.at(-1) || null
}

export type ChipId = 'openFolder' | 'sample' | 'explain' | 'security' | 'review' | 'conflicts' | 'lastCommit' | 'bugs' | 'tests' | 'improve'

export interface HomeChip {
  id: ChipId
  label: string
  /** Taslağa yazılacak istem; eylem çiplerinde (klasör aç) yok */
  prompt?: string
}

export interface HomeSignals {
  root: string | null
  /** git deposu mu; bilinmiyorsa null (henüz yüklenmedi) */
  isRepo: boolean | null
  dirty: number
  conflicts: number
  /** Deterministik güvenlik denetiminin bulgu sayısı (kor audit --security); bilinmiyorsa null */
  securityFindings: number | null
}

/**
 * §23.3: proje yoksa "Klasör aç", "Örnek projeyle dene"; proje varsa graf/analiz/git'ten türetilmiş öneriler önce,
 * genel öneriler sonra. En çok 5 çip.
 */
export function homeChips(s: HomeSignals): HomeChip[] {
  if (!s.root) {
    return [
      { id: 'openFolder', label: 'Klasör aç' },
      { id: 'sample', label: 'Örnek projeyle dene' },
    ]
  }
  const out: HomeChip[] = []
  if (s.conflicts > 0) out.push({ id: 'conflicts', label: `${s.conflicts} çakışmayı çöz`, prompt: 'Git birleştirme çakışmalarını incele ve her dosya için çözüm öner.' })
  if (s.securityFindings !== null && s.securityFindings > 0) {
    out.push({
      id: 'security',
      label: `${s.securityFindings} güvenlik bulgusunu incele`,
      prompt: '`kor audit --security` bulgularını dosya:satır ve kural kimliğiyle açıkla, her biri için düzeltme öner.',
    })
  }
  if (s.dirty > 0) out.push({ id: 'review', label: `${s.dirty} değişikliği gözden geçir`, prompt: 'Kaydedilmemiş değişiklikleri gözden geçir: hata, eksik test ve risk var mı?' })
  if (s.isRepo) out.push({ id: 'lastCommit', label: "Son commit'i açıkla", prompt: "Son commit'i (git show HEAD) açıkla: ne değişti ve neden?" })
  out.push(
    { id: 'explain', label: 'Projeyi açıkla', prompt: 'Bu projenin yapısını ve ne yaptığını kısaca açıkla.' },
    { id: 'bugs', label: 'Hata bul', prompt: 'Kod tabanındaki olası hataları bul ve listele.' },
    { id: 'tests', label: 'Test yaz', prompt: 'Test kapsamı en zayıf modül için birim testleri yaz.' },
    { id: 'improve', label: 'İyileştir', prompt: 'Kodu okunabilirlik ve performans açısından gözden geçir, iyileştirme öner.' },
  )
  return out.slice(0, 5)
}

/** "Beta · Sürüme az kaldı" yerine gerçek kanal ve sürüm: "0.8.0 · beta". */
export function releaseLabel(version: string | null, channel: string | null | undefined): string | null {
  if (!version) return null
  const ch = channel && channel !== 'stable' ? ` · ${channel}` : ''
  return `${version}${ch}`
}
