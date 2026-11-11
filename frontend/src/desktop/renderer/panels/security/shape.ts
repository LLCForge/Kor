// Güvenlik paneli: saf yardımcılar (test: shape.test.ts).
import type { AuditEntry, AuditVerify } from '../../../ipc-security.ts'

export const AUDIT_PAGE = 50

const DECISIONS: Record<NonNullable<AuditEntry['decision']>, string> = {
  allow: 'kuralla izin',
  deny: 'kuralla reddedildi',
  approved: 'kullanıcı onayladı',
  rejected: 'kullanıcı reddetti',
}

export function decisionLabel(d: AuditEntry['decision']): string {
  return d ? DECISIONS[d] : ''
}

/** Satırın tek satırlık özeti (model ya da kullanıcı içeriği: arayüzde translate="no"). */
export function entrySummary(e: AuditEntry): string {
  if (e.action === 'tool') return [e.tool, e.target].filter(Boolean).join(' · ')
  if (e.action === 'model') {
    const cost = e.cost_usd ? ` · ${e.cost_estimated ? '≈' : ''}$${Number(e.cost_usd).toFixed(4)}` : ''
    const masked = e.masked ? ` · ${e.masked} sır maskelendi` : ''
    return `${e.input ?? 0} → ${e.output ?? 0} token${cost}${masked}`
  }
  if (e.action === 'task') return (e.text ?? '').split('\n', 1)[0] ?? ''
  return e.target ?? e.text ?? ''
}

/** `[vault.scope]` değerinin okunur hâli: null = tüm komutlar, [] = hiçbir komut. */
export function scopeLabel(scope: string[] | null | undefined): string {
  if (scope === null || scope === undefined) return 'tüm komutlar'
  return scope.length === 0 ? 'hiçbir komut' : scope.join(', ')
}

/** Sayfa sınırları; `from`/`to` 1'den başlar, boş listede 0. */
export function pageInfo(total: number, offset: number, limit: number) {
  const from = total === 0 ? 0 : Math.min(offset + 1, total)
  const to = Math.min(offset + limit, total)
  return { from, to, hasPrev: offset > 0, hasNext: offset + limit < total }
}

const REASONS: Record<NonNullable<AuditVerify['reason']>, string> = {
  json: 'satır geçerli JSON değil',
  prev: 'önceki kaydın özeti eşleşmiyor (satır silinmiş ya da yer değiştirmiş)',
  hash: 'satırın özeti içeriğiyle eşleşmiyor (satır değiştirilmiş)',
}

const HEADS: Record<AuditVerify['head'], string> = {
  ok: '',
  behind: ' Son yazma baş dosyasını güncelleyemedi; bir sonraki kayıt düzeltir.',
  missing: ' Baş dosyası henüz yok: bir sonraki kayda kadar sondan silinen kayıtlar fark edilemez.',
  truncated: '',
}

export function verifyText(v: AuditVerify): string {
  const trimmed = v.trimmed ? ' Eski arşivler silinmiş; zincir kalan en eski dosyadan denetlendi.' : ''
  if (v.reason) return `Zincir ${v.file ? `${v.file} dosyasında ` : ''}${v.line ?? '?'}. satırda bozuk: ${REASONS[v.reason]}.`
  if (v.head === 'truncated') return 'Kaydın sonundan girdiler silinmiş ya da yeniden yazılmış: baş dosyasının onayladığı kayıt bulunamadı.'
  if (v.entries === 0) return 'Kayıt yok; doğrulanacak zincir boş.'
  const files = v.files > 1 ? ` (${v.files} dosya)` : ''
  return `Zincir sağlam: ${v.entries} kaydın hepsi doğrulandı${files}.${HEADS[v.head]}${trimmed}`
}

/** `kor_secrets::vault::valid_name` ile aynı kural. */
export function validVaultName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(name)
}
