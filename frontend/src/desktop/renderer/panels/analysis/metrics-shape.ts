// Kod metrikleri bölümlerinin saf yardımcıları (ADR 0047): etiketler, kaynak/formül metinleri, biçimleme.
// React yok; node --test ile denenir.
import type { DeadItem, Dimension, DimensionKey, GrowthBasis, TestBasis } from '../../../ipc-metrics.ts'
import { uiLocale, percentText } from '../../ui-locale.ts'

export const DIM_LABEL: Record<DimensionKey, string> = {
  maintainability: 'Bakım kolaylığı',
  coverage: 'Test kapsamı',
  documentation: 'Dokümantasyon',
  security: 'Güvenlik',
  performance: 'Performans',
}

export const FORMULA: Record<string, string> = {
  maintainability: '100 × (0,5 × (1 − min(1, 4 × karmaşıklığı > 10 olan fonksiyon oranı)) + 0,25 × (1 − min(1, 4 × 500 kod satırını aşan dosya oranı)) + 0,25 × (1 − min(1, 5 × tekrarlanan satır oranı)))',
  coverage: 'satır kapsamı %',
  documentation: '80 × belgelenmiş public öğe oranı + 10 × README + 10 × ADR klasörü',
  'documentation-nopublic': '50 × README + 50 × ADR klasörü (public öğe yok)',
  security: '100 − 25 × kritik − 10 × yüksek − 3 × orta − 1 × düşük',
  performance: '100 − 2 × temel ölçüme göre süre gerilemesi %',
}

export const NOTE: Record<string, string> = {
  'no-source': 'Kaynak dosya bulunamadı.',
  'no-coverage': 'Kapsam raporu bulunamadı (lcov.info, cobertura/coverage.xml, coverage.json, coverage.out, tarpaulin-report.json). Kor kapsam araçlarını kendisi çalıştırmaz.',
  'coverage-stale': 'Kapsam raporu son commit’ten eski.',
  'no-perf': 'Bu proje için `kor perf` verisi yok.',
  'perf-unreadable': 'En yeni `kor perf` çalışması okunamadı.',
  'perf-no-baseline': 'En yeni `kor perf` çalışmasında temel karşılaştırma yok; karşılaştırma olmadan not verilmez.',
}

export const METRIC_LABEL: Record<string, string> = {
  functions: 'Fonksiyon',
  avg_complexity: 'Ortalama karmaşıklık',
  complex_share: 'Karmaşıklığı > 10 olan fonksiyonlar',
  large_file_share: '500 kod satırını aşan dosyalar',
  duplication: 'Tekrarlanan satırlar',
  line_coverage: 'Satır kapsamı',
  covered_lines: 'Kapsanan satırlar',
  coverable_lines: 'Kapsanabilir satırlar',
  public_items: 'Public öğeler',
  documented_public: 'Belgelenmiş public öğeler',
  readme: 'README',
  adr: 'ADR klasörü',
  critical: 'Kritik bulgular',
  high: 'Yüksek bulgular',
  medium: 'Orta bulgular',
  low: 'Düşük bulgular',
  wall_ms: 'Süre (ms)',
  hottest_share: 'En sıcak fonksiyonun payı',
  wall_change: 'Temel ölçüme göre süre değişimi',
}

export const DEBT_FORMULA =
  'saat ≈ (0,25 × Σ max(0, karmaşıklık − 10) + 0,5 × max(0, kod satırı − 400) / 100 + tekrarlanan satır / 40) × (1 + min(180 gündeki commit, 20) / 10) × test çarpanı (kapsam: 1 + 0,5 × kapsanmayan oran; rapor yoksa: test varsa 1,0, yoksa 1,5)'
export const IMPACT_FORMULA = 'saat ≈ 0,5 × değişen dosya + 0,2 × etkilenen dosya + 0,5 × uç nokta + 1,0 × migration + 0,5 × (paket − 1)'

/** Bir ondalık, Türkçe ayraçla; tam sayıysa ondalıksız. */
export function num(v: number): string {
  const r = Math.round(v * 10) / 10
  return Number.isInteger(r) ? r.toLocaleString(uiLocale()) : r.toLocaleString(uiLocale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 })
}

export function signed(v: number): string {
  return v > 0 ? `+${num(v)}` : num(v)
}

export function day(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toISOString().slice(0, 10)
}

export function formulaFor(d: Dimension): string {
  if (d.key === 'documentation' && d.metrics[0]?.value === 0) return FORMULA['documentation-nopublic'] ?? ''
  return FORMULA[d.key] ?? d.formula
}

export function metricValue(unit: string, value: number): string {
  if (unit === 'bool') return value > 0 ? 'var' : 'yok'
  return unit === '%' ? percentText(num(value)) : num(value)
}

export function testBasis(t: TestBasis): string {
  switch (t.kind) {
    case 'coverage':
      return `kapsam %${num(t.percent)} (${t.source})`
    case 'not_in_report':
      return `${t.source} içinde yok`
    case 'tests_found':
      return 'test bulundu'
    case 'no_tests':
      return 'test bulunamadı'
  }
}

export function growthBasis(g: GrowthBasis): string {
  return g.kind === 'snapshots' ? `${g.count} kalite anlık görüntüsü, ${g.from} … ${g.to}` : `30 gün önceki ${g.rev} commit’i ile HEAD, ikisi de kapsamsız`
}

export function deadWhy(d: DeadItem): string {
  switch (d.reason.kind) {
    case 'unreachable':
      return d.unchanged_days === null ? 'hiçbir giriş noktası erişmiyor' : `hiçbir giriş noktası erişmiyor; dosya ${d.unchanged_days} gündür değişmedi`
    case 'name_elsewhere':
      return `adı grafın çözemediği yerlerde ${d.reason.count} kez geçiyor`
    case 'reflection':
      return 'dosyası kodu adla yüklüyor (reflection)'
  }
}

export const CONFIDENCE_LABEL: Record<DeadItem['confidence'], string> = { high: 'Yüksek', medium: 'Orta', ambiguous: 'Belirsiz' }

export const PRUNE_NOTE: Record<string, string> = {
  'prune-nothing': 'Bu güven düzeyinde silinecek bir şey yok.',
  'prune-no-checker': 'Derleyici ya da tür denetleyici bulunamadı; bir denetim komutu girin.',
  'prune-all-dirty': 'Aday dosyaların hepsinde commit edilmemiş değişiklik var.',
  'prune-check-failed': 'Silme denetleyiciden geçmedi; hiçbir şey commit edilmedi.',
  'prune-tests-failed': 'Silmeden sonra testler başarısız oldu; hiçbir şey commit edilmedi.',
  'prune-no-tests': 'Test komutu bulunamadı; silmeyi yalnızca denetleyici doğruladı.',
}

export function pruneNote(n: string): string {
  return PRUNE_NOTE[n] ?? n
}
