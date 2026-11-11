import type { KorApi } from '../ipc.ts'
import { createKorBridge } from './kor-bridge.ts'

declare global {
  interface Window {
    kor: KorApi
  }
}

// İlk içe aktaran köprüyü kurar; içe aktarma sırasından bağımsızdır.
window.kor ??= createKorBridge()

export const api = window.kor

/** Rust komut hata metnini arayüzde gösterilecek hale getirir. */
export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}
