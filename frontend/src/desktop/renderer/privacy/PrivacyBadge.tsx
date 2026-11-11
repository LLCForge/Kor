// Model seçicideki gizlilik rozeti (§31.2, ADR 0057): eğitim · saklama · bölge; fareyle üzerine
// gelince tam kart (alt işleyici, ZDR, KVKK/GDPR notu, kaynak bağlantısı ve tarihi). Kartlar tek
// istekle gelir ve önbellekte kalır.
import { ShieldCheck, ShieldQuestion, ShieldAlert } from 'lucide-react'
import useSWR from 'swr'
import type { PrivacyApi } from '../../ipc.gen.ts'
import { api } from '../api.ts'
import { badge, rowFor } from './shape.ts'
import './privacy.css'

export const privacyApi = (api as unknown as { privacy: PrivacyApi }).privacy

export const PRIVACY_KEY = 'privacy:cards'

/** Tüm sağlayıcıların kartları (seçici açıkken bir kez). */
export function usePrivacyCards(enabled: boolean) {
  return useSWR(enabled ? PRIVACY_KEY : null, () => privacyApi.cards(), { revalidateOnFocus: false })
}

export function PrivacyBadge({ modelId, cards }: { modelId: string; cards: ReturnType<typeof usePrivacyCards>['data'] }) {
  const row = rowFor(cards, modelId)
  if (!row) return null
  const b = badge(row, modelId)
  if (!b.label) return null
  const Icon = b.tone === 'ok' ? ShieldCheck : b.tone === 'unknown' ? ShieldQuestion : ShieldAlert
  return (
    <span className={`kp-badge kp-${b.tone}`} title={b.title} data-kor-cmd="privacy.cards">
      <Icon size={12} aria-hidden="true" />
      <span className="sr-only">Gizlilik kartı: </span>
      {b.label}
    </span>
  )
}

/** Satırın soluk olup olmayacağı (şart karşılanmıyor). */
export function isDimmed(cards: ReturnType<typeof usePrivacyCards>['data'], modelId: string): boolean {
  const row = rowFor(cards, modelId)
  return row ? badge(row, modelId).dimmed : false
}
