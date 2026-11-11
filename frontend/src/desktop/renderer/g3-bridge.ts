// [g3] Akış kartlarının App'e ulaştığı köprü.
import { createContext, useContext } from 'react'
import type { InteractAnswer } from '../../core/interactive.ts'

export interface G3Actions {
  /** Kullanıcı adına mesaj gönderir (öneri düğmeleri, yeniden dene) */
  send(text: string, attachments?: string[]): void
  /** line: g4 önizlemede satıra kaydırır */
  openFile(path: string, line?: number): void
  /** Sağ paneli değişiklikler sekmesinde açar */
  openChanges(): void
  /** Bekleyen soru/plan kartını cevaplar */
  answer(callId: string, answer: InteractAnswer): Promise<void>
  /** Dosya ağacını ve önizlemeleri yeniler (kod kaydedildikten sonra) */
  refreshFiles(): void
  /** Etkin sohbetin öğe zaman damgaları (öğe kimliği → epoch ms) */
  times: Readonly<Record<string, number>>
}

/**
 * App her çizimde (çocuğu Thread çizilmeden önce) günceller; kartlar olay anında, Thread zaman damgaları için okur.
 * Bağlam sağlayıcı yerine bu yol seçildi: App.tsx'teki ortak Thread satırına dokunmadan bağlanır.
 */
export const g3Actions: { current: G3Actions | null } = { current: null }

/** Yan bölmedeki Thread'in kendi sohbetine bağlı eylemleri; ana bölmede yoktur. */
export type G3PaneActions = Pick<G3Actions, 'send' | 'answer' | 'times'>
export const G3PaneContext = createContext<G3PaneActions | null>(null)

/** Kartlar eylemleri bununla alır: yan bölmedeki kart ana bölmenin sohbetine cevap vermesin. */
export function useG3Actions(): () => G3Actions | null {
  const pane = useContext(G3PaneContext)
  return () => (g3Actions.current && pane ? { ...g3Actions.current, ...pane } : g3Actions.current)
}
