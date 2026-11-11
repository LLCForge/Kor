import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { userConfigDir } from './layout-g28.ts'

/**
 * Tüm sohbetlerde ve projelerde geçerli kalıcı hafıza (Claude'un memory'si gibi).
 * Yol her erişimde kullanıcı veri klasöründen hesaplanır (KOR_HOME varsa o): testler gerçek hafızaya dokunamaz; sürüm/kurulum/taşınabilirlik değişse de hafıza aynı dosyada kalır.
 */
export function memoryFile(): string {
  return path.join(userConfigDir(), 'memory.md')
}
/** Geriye uyumlu sabit: açılışta okunan değer (KOR_HOME verilmişse oradan). */
export const MEMORY_FILE = memoryFile()
export const MAX_MEMORY_CHARS = 20_000

/**
 * Kurum belleği: yöneticinin makine genelinde koyduğu, salt okunur talimat/bilgi dosyası (uygulama asla yazmaz).
 * Katman sırası: kurum > kullanıcı (memory.md) > proje (.kor/knowledge.json) > oturum. KOR_ORG_MEMORY ile değiştirilebilir.
 */
export const ORG_MEMORY_FILE =
  process.env.KOR_ORG_MEMORY ||
  (process.platform === 'win32'
    ? path.join(process.env.ProgramData || 'C:\\ProgramData', 'Kor', 'memory.md')
    : process.platform === 'darwin'
      ? '/Library/Application Support/Kor/memory.md'
      : '/etc/kor/memory.md')

export async function loadMemory(file: string = memoryFile()): Promise<string | null> {
  try {
    return (await readFile(file, 'utf8')).trim() || null
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

/** Boş metin hafızayı siler. */
export async function saveMemory(text: string, file: string = memoryFile()): Promise<void> {
  if (text.length > MAX_MEMORY_CHARS) throw new Error(`Hafıza en fazla ${MAX_MEMORY_CHARS} karakter olabilir`)
  if (!text.trim()) {
    await rm(file, { force: true })
    return
  }
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${text.trimEnd()}\n`, { encoding: 'utf8', mode: 0o600 })
}

/** remember aracının kullandığı ekleme: "- [2026-10-27] not" */
export async function appendMemory(note: string, file: string = memoryFile()): Promise<void> {
  const line = `- [${new Date().toISOString().slice(0, 10)}] ${note.replace(/\s+/g, ' ').trim()}`
  const current = (await loadMemory(file)) ?? ''
  const next = current ? `${current}\n${line}` : line
  if (next.length > MAX_MEMORY_CHARS) {
    throw new Error('Hafıza dolu. Kullanıcıdan eski notları Hafıza ekranından temizlemesini iste.')
  }
  await saveMemory(next, file)
}
