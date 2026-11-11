// [g10] Atomik dosya yazımı: içerik önce aynı klasördeki geçici dosyaya yazılır, sonra tek adımda yerine taşınır.
// Çökme/elektrik kesintisinde hedef dosya ya eski ya yeni haliyle kalır; yarım yazılmış dosya oluşmaz.
import { randomBytes } from 'node:crypto'
import { realpath, rename, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { backoffDelay, sleep } from './resilience-g10.ts'

// Windows'ta virüs tarayıcı/dizinleyici hedefi kısa süre kilitleyebilir; taşıma birkaç kez denenir.
const RENAME_RETRY_CODES = new Set(['EPERM', 'EACCES', 'EBUSY'])
const RENAME_ATTEMPTS = 6

async function targetPath(file: string): Promise<string> {
  try {
    // Sembolik bağlantı yerine gösterdiği dosya güncellenir; aksi halde bağlantı düz dosyayla değişirdi.
    return await realpath(file)
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return file
    throw err
  }
}

export async function atomicWriteFile(file: string, content: string): Promise<void> {
  const target = await targetPath(file)
  const mode = await stat(target).then(
    (s) => s.mode & 0o777,
    () => undefined,
  )
  const tmp = path.join(path.dirname(target), `.${path.basename(target)}.${randomBytes(6).toString('hex')}.tmp`)
  await writeFile(tmp, content, { encoding: 'utf8', flag: 'wx', ...(mode !== undefined ? { mode } : {}) })
  try {
    for (let attempt = 0; ; attempt++) {
      try {
        await rename(tmp, target)
        return
      } catch (err) {
        const code = (err as NodeJS.ErrnoException).code
        if (!code || !RENAME_RETRY_CODES.has(code)) throw err
        if (attempt + 1 >= RENAME_ATTEMPTS) {
          // Hedefi silmeye izin vermeyen bir süreç dosyayı açık tutuyor (bazı editörler/eşitleme araçları):
          // değişiklik kaybolmasın diye yerinde yazmaya düşülür; bu yazma atomik değildir.
          console.warn('Atomik taşıma başarısız, yerinde yazılıyor:', target, code)
          await writeFile(target, content, 'utf8')
          await rm(tmp, { force: true })
          return
        }
        await sleep(50 + backoffDelay(attempt, 50, 800))
      }
    }
  } catch (err) {
    await rm(tmp, { force: true }).catch((cleanupErr: unknown) => console.warn('Geçici dosya silinemedi:', tmp, cleanupErr))
    throw err
  }
}
