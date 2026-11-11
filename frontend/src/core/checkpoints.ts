// Kontrol noktaları: her kullanıcı turundan önce açılır, turda değişen dosyaların tur öncesi hali saklanır.
// Bellekte ve oturum başınadır; uygulama kapanınca kaybolur.
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { isSensitivePath } from './permissions.ts'
import { resolveReal } from './safe-path.ts'

export interface CheckpointInfo {
  id: string
  /** Kullanıcı mesajının session.messages içindeki konumu; konuşmayı bu noktadan kesmek için */
  messageIndex: number
  /** Kullanıcının yazdığı metin (ekler hariç); arayüz mesajla eşleştirmek için kullanır */
  text: string
  createdAt: number
  files: string[]
  /** false: bu nokta veya sonraki bir nokta bellek sınırını aştı, geri dönülemez */
  available: boolean
}

interface Checkpoint {
  id: string
  messageIndex: number
  text: string
  createdAt: number
  /** Dosyanın bu noktadaki hali; null = dosya yoktu */
  files: Map<string, string | null>
  chars: number
  incomplete: boolean
}

/** Oturum başına saklanan toplam içerik (UTF-16 karakter; ~64 MB). */
export const MAX_CHECKPOINT_CHARS = 32_000_000
export const MAX_CHECKPOINTS = 100

export class CheckpointStore {
  private readonly points: Checkpoint[] = []
  private chars = 0
  private readonly maxChars: number
  private readonly maxPoints: number

  constructor(maxChars = MAX_CHECKPOINT_CHARS, maxPoints = MAX_CHECKPOINTS) {
    this.maxChars = maxChars
    this.maxPoints = maxPoints
  }

  begin(id: string, messageIndex: number, text: string, now = Date.now()): void {
    this.points.push({ id, messageIndex, text, createdAt: now, files: new Map(), chars: 0, incomplete: false })
    while (this.points.length > this.maxPoints) this.dropOldest()
  }

  /** Dosyanın son noktadaki ilk halini saklar; aynı turdaki sonraki düzenlemeler bu kaydı değiştirmez. */
  record(path: string, before: string | null): void {
    const cp = this.points.at(-1)
    if (!cp || cp.incomplete || cp.files.has(path)) return
    const size = before?.length ?? 0
    cp.files.set(path, before)
    cp.chars += size
    this.chars += size
    while (this.chars > this.maxChars && this.points.length > 1) this.dropOldest()
    if (this.chars > this.maxChars) {
      // Tek başına sınırı aşan tur: eksik kayıtla geri sarmak dosyaları tutarsız bırakır, nokta kullanılamaz olur.
      this.chars -= cp.chars
      cp.files.clear()
      cp.chars = 0
      cp.incomplete = true
    }
  }

  list(): CheckpointInfo[] {
    return this.points.map((p, i) => ({
      id: p.id,
      messageIndex: p.messageIndex,
      text: p.text,
      createdAt: p.createdAt,
      files: [...this.plan(i).keys()].sort(),
      available: !this.points.slice(i).some((q) => q.incomplete),
    }))
  }

  get(id: string): CheckpointInfo | undefined {
    return this.list().find((p) => p.id === id)
  }

  /** K noktasına dönmek için her dosyanın hedef hali: K ve sonrasındaki en erken kayıt. */
  restorePlan(id: string): Map<string, string | null> {
    const index = this.indexOf(id)
    if (this.points.slice(index).some((p) => p.incomplete)) {
      throw new Error('Bu kontrol noktası bellek sınırını aşan bir tur içerdiği için geri yüklenemiyor.')
    }
    return this.plan(index)
  }

  /** K ve sonraki noktaları siler (geri sarma tamamlandıktan sonra). */
  dropFrom(id: string): void {
    const index = this.indexOf(id)
    for (const p of this.points.splice(index)) this.chars -= p.chars
  }

  /** Kalan noktalarda dosyanın en erken kaydı; undefined = hiç kaydedilmemiş. */
  earliest(path: string): string | null | undefined {
    for (const p of this.points) if (p.files.has(path)) return p.files.get(path)
    return undefined
  }

  private plan(index: number): Map<string, string | null> {
    const files = new Map<string, string | null>()
    for (const p of this.points.slice(index)) {
      for (const [path, content] of p.files) if (!files.has(path)) files.set(path, content)
    }
    return files
  }

  private indexOf(id: string): number {
    const index = this.points.findIndex((p) => p.id === id)
    if (index < 0) throw new Error('Kontrol noktası bulunamadı (bellek sınırı nedeniyle silinmiş olabilir).')
    return index
  }

  private dropOldest(): void {
    const old = this.points.shift()
    if (old) this.chars -= old.chars
  }
}

export async function readIfExists(abs: string): Promise<string | null> {
  try {
    return await readFile(abs, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
}

/**
 * Plandaki dosyaları hedef haline getirir (null = sil). Zaten hedefte olanlara dokunulmaz.
 * Bir dosya başarısız olsa da diğerleri denenir; işlem tekrarlanabilir.
 * beforeWrite: yazmadan hemen önce (izleyici kendi yazdığımızı harici değişiklik sanmasın diye).
 */
export async function restoreFiles(
  root: string,
  plan: ReadonlyMap<string, string | null>,
  beforeWrite?: (rel: string, target: string | null) => void,
): Promise<{ restored: string[]; failures: string[] }> {
  const restored: string[] = []
  const failures: string[] = []
  for (const [rel, target] of plan) {
    try {
      if (isSensitivePath(rel)) throw new Error('hassas dosya')
      const abs = await resolveReal(root, rel)
      beforeWrite?.(rel, target)
      if ((await readIfExists(abs)) === target) continue
      if (target === null) {
        await rm(abs, { force: true })
      } else {
        await mkdir(path.dirname(abs), { recursive: true })
        await writeFile(abs, target, 'utf8')
      }
      restored.push(rel)
    } catch (err) {
      console.error('Geri sarma: dosya geri yüklenemedi', rel, err)
      failures.push(`${rel} (${err instanceof Error ? err.message : String(err)})`)
    }
  }
  return { restored, failures }
}

/**
 * Geri sarmadan sonra "oturumda değişen dosyalar" (originals) haritasını günceller:
 * dosya oturumun ilk haline döndüyse listeden çıkar; listede yoksa ama hâlâ farklıysa ilk hali geri eklenir.
 */
export function reconcileOriginals(
  originals: Map<string, string | null>,
  restored: ReadonlyMap<string, string | null>,
  store: CheckpointStore,
): void {
  for (const [path, content] of restored) {
    const original = originals.has(path) ? originals.get(path) : store.earliest(path)
    if (original === undefined || original === content) originals.delete(path)
    else originals.set(path, original)
  }
}
