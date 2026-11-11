// §27.3 "Buradan dallan": arayüzdeki mesajı kayıtlı dökümdeki mesaj numarasına eşler. Döküm araç
// sonuçlarını da içerdiği için sıra numarası değil, rol + metin (ve aynı metnin kaçıncı tekrarı) kullanılır.
import type { ForkMessage } from '../../ipc-ek14.ts'

const norm = (t: string) => t.replace(/\s+/g, ' ').trim().slice(0, 160)

/**
 * `role`/`text` mesajının dökümdeki numarası; `nth` aynı rol+metnin kaçıncı geçişi (0'dan). Bulunamazsa null.
 * Yanıt mesajları dökümde araç çağrılarıyla bölünebilir: metnin başı eşleşmesi yeterli.
 */
export function messageNumber(messages: readonly ForkMessage[], role: 'user' | 'assistant', text: string, nth: number): number | null {
  const want = norm(text)
  if (!want) return null
  const hits = messages.filter((m) => {
    if (m.role !== role) return false
    const have = norm(m.text)
    return have.length > 0 && (have === want || want.startsWith(have) || have.startsWith(want))
  })
  return hits[nth]?.n ?? hits[hits.length - 1]?.n ?? null
}
