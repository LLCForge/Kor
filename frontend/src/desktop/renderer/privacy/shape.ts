// Sağlayıcı gizlilik kartı (§31.2, ADR 0057): model kimliğinden kartı bulur, rozet metnini ve
// soluklaştırmayı hesaplar. DOM yok; düğüm testleri çalıştırır. Bilinmeyen alan "bilinmiyor" kalır.
import type { PrivacyCards, PrivacyRow } from '../../ipc.gen.ts'

/** `openrouter/anthropic/claude…` → `openrouter`; ön eki olmayan kimlik sağlayıcısız sayılır. */
export function providerOf(modelId: string): string | null {
  const i = modelId.indexOf('/')
  return i > 0 ? modelId.slice(0, i) : null
}

export function rowFor(cards: PrivacyCards | undefined, modelId: string): PrivacyRow | null {
  const p = providerOf(modelId)
  if (!cards || !p) return null
  return cards.providers.find((r) => r.card.provider === p) ?? null
}

/** Ağ geçidinin arkasındaki gerçek işleyici: model kimliğindeki üretici. */
export function subProcessor(row: PrivacyRow, modelId: string): string | null {
  if (!row.card.gateway) return null
  const rest = modelId.split('/').slice(1)
  return rest.length > 1 ? rest[0] : null
}

export interface Badge {
  /** Kısa rozet: bilinen eğitim · saklama · bölge değerleri (hiçbiri bilinmiyorsa boş) */
  label: string
  /** Şart karşılanmıyor: seçicide soluk, yönlendirici atlar */
  dimmed: boolean
  /** Fareyle üzerine gelince tam kart */
  title: string
  tone: 'ok' | 'warn' | 'unknown'
}

export function badge(row: PrivacyRow, modelId: string): Badge {
  const t = row.text
  const sub = subProcessor(row, modelId) ?? (t.subProcessor !== '-' ? t.subProcessor : null)
  const lines = [
    `Eğitimde kullanım: ${t.training}`,
    `Veri saklama: ${t.retention}`,
    `Bölge: ${t.region}`,
    `ZDR anlaşması gerekli mi: ${t.zdr}`,
    ...(sub ? [`Alt işleyici: ${sub}`] : []),
    `KVKK/GDPR: ${t.law}`,
    ...row.card.notes,
    ...row.card.sources.map((s) => `Kaynak: ${s.url} (${s.retrieved})`),
    ...(row.unmet.length ? [`Karşılanmayan şart: ${row.unmet.join(', ')}`] : []),
  ]
  const unknown = row.card.training === 'unknown' && row.card.region === 'unknown' && row.card.retention === 'unknown'
  const tone: Badge['tone'] = row.card.local || (row.card.training === 'no' && row.card.zdr !== 'required') ? 'ok' : unknown ? 'unknown' : 'warn'
  return {
    // Yalnızca bilinen alanlar; üçü de bilinmiyorsa etiket boş kalır ve rozet çizilmez.
    label: ([[row.card.training, t.training], [row.card.retention, t.retention], [row.card.region, t.region]] as const)
      .filter(([v]) => v !== 'unknown')
      .map(([, text]) => text)
      .join(' · '),
    dimmed: row.unmet.length > 0,
    title: lines.join('\n'),
    tone,
  }
}
