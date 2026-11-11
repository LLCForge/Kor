// [g6] Sıkıştırılmış geçmişin arayüzde katlanması.
// Sorun: compaction yalnızca modele giden diziyi küçültüyordu; arayüzde ilk mesaja kadar yüzlerce blok DOM'da kalıyor,
// sohbet kasıyordu. Burada sıkıştırma ayracından ÖNCEKİ bloklar akıştan çıkarılır. Veri silinmez: sohbet kaydı ve
// modele giden özet yerinde durur, kullanıcı "önceki kısmı göster" ile açar.
//
// Kural: EN SON sıkıştırma ayracından önceki her şey (önceki ayraçlar dahil) katlanır. Böylece 5 sıkıştırma birikse de
// tek bir katlama noktası olur; kullanıcı açtığında hepsi görünür.
import { isLiveItem, type ChatItem } from './chat-state.ts'

export interface FoldedRows {
  /** Ekranda çizilecek item'lar: son sıkıştırma ayracı ve sonrası, açılmışsa hepsi. */
  visible: ChatItem[]
  /** Son sıkıştırma ayracının kimliği (kapalıysa "N eski blok" düğmesi buna bağlanır) */
  foldId: string | null
  /** Katlanan (DOM'a girmeyen) item sayısı */
  hiddenCount: number
}

/**
 * Son sıkıştırma ayracından önceki item'ları katlar. `revealId` o ayracın kimliğiyse (kullanıcı açtıysa) hiçbir şey
 * katlanmaz. Sıkıştırma yoksa dizi olduğu gibi döner.
 */
export function rowsToFold(items: readonly ChatItem[], revealId: string | null): FoldedRows {
  const foldIndex = items.findLastIndex((i) => i.kind === 'compaction')
  if (foldIndex < 0) return { visible: items as ChatItem[], foldId: null, hiddenCount: 0 }
  const fold = items[foldIndex]
  if (!fold) return { visible: items as ChatItem[], foldId: null, hiddenCount: 0 }
  if (revealId === fold.id) return { visible: items as ChatItem[], foldId: fold.id, hiddenCount: 0 }
  // Ayracın kendisi kalır: kullanıcı "N eski blok" düğmesini orada görür.
  // Süren veya onay bekleyen kart katlanmaz: arka plan ajanı onay beklerken kart gizli kalırsa ajan askıda kalır.
  const live = items.slice(0, foldIndex).filter(isLiveItem)
  return { visible: [...live, ...items.slice(foldIndex)], foldId: fold.id, hiddenCount: foldIndex - live.length }
}
