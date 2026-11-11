// [g33] Ajanın geçici/deneme dosyaları için scratch dizini: iş akışı klasörünü boş test dosyalarıyla kirletmesin diye,
// kalıcı olması gerekmeyen her şey (deneme betikleri, prob dosyaları, tek seferlik çıktılar) buraya yazılır. Claude Code'un
// scratchpad'i gibi, işletim sistemi temp'i altında ve çalışma alanına göre ayrı tutulur.
import { createHash } from 'node:crypto'
import { mkdirSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/** Çalışma alanına özel scratch dizini (OS temp altında); yoksa oluşturur ve mutlak yolu döndürür. */
export function scratchDir(root: string): string {
  const key = createHash('sha256').update(path.resolve(root)).digest('hex').slice(0, 16)
  const dir = path.join(os.tmpdir(), 'kor-scratch', key)
  try {
    mkdirSync(dir, { recursive: true })
  } catch {
    /* oluşturulamadıysa yol yine döner; yazım anında gerçek hata kullanıcıya görünür */
  }
  return dir
}
