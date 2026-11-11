// Toplu düzenleme önizleme tablosu ve ~/Kor özeti için saf yardımcılar (node --test ile denenir).
import type { BatchOp, BatchRow } from '../../ipc-batch.ts'
import type { KorFolderPass } from '../../ipc-korfolder.ts'

export const OP_LABEL: Readonly<Record<BatchOp, string>> = {
  rename: 'Yeniden adlandır',
  convert_image: 'Görsel dönüştür',
  strip_metadata: 'Meta veriyi sil',
  replace: 'Bul ve değiştir',
  reencode: 'Kodlamayı düzelt',
  csv_clean: 'CSV temizle',
  pdf_merge: 'PDF birleştir',
  media: 'Ses/video dönüştür',
}

export interface PreviewRow {
  key: string
  op: string
  from: string
  to: string
  detail: string
  /** Yeni dosya yerine mevcut dosya değişiyor (geri alma günlüğünde saklanır). */
  inPlace: boolean
}

/** `base` altındaki yolları kısaltır; ayırıcıyı `/` yapar. */
export function shortPath(path: string, base: string | null): string {
  const p = path.replaceAll('\\', '/')
  if (!base) return p
  const b = base.replaceAll('\\', '/').replace(/\/+$/, '')
  const lower = p.toLocaleLowerCase('en')
  const prefix = `${b.toLocaleLowerCase('en')}/`
  return lower.startsWith(prefix) ? p.slice(prefix.length) : p
}

/** Ortak klasör: seçili dosyaların ilkinin klasörü, hepsi onun altındaysa. */
export function commonDir(files: readonly string[]): string | null {
  const first = files[0]
  if (first === undefined) return null
  const norm = first.replaceAll('\\', '/')
  const dir = norm.slice(0, norm.lastIndexOf('/'))
  if (!dir) return null
  const prefix = `${dir.toLocaleLowerCase('en')}/`
  return files.every((f) => f.replaceAll('\\', '/').toLocaleLowerCase('en').startsWith(prefix)) ? dir : null
}

export function previewRows(rows: readonly BatchRow[], base: string | null): PreviewRow[] {
  return rows.map((r, i) => {
    const from = r.from.split(', ').map((f) => shortPath(f, base)).join(', ')
    const to = shortPath(r.to, base)
    return {
      key: `${i}:${r.to}`,
      op: OP_LABEL[r.op] ?? r.op,
      from,
      to,
      detail: r.detail,
      inPlace: r.from === r.to,
    }
  })
}

/** Watcher geçişinin bir satırlık özeti; değişiklik yoksa null. */
export function passSummary(p: KorFolderPass): string | null {
  const parts: string[] = []
  if (p.sync.written.length) parts.push(`${p.sync.written.length} görünüm güncellendi`)
  if (p.sync.applied.length) parts.push(`${p.sync.applied.length} düzenleme kayda işlendi`)
  const done = p.tasks.filter((t) => t.status === 'done').length
  const failed = p.tasks.length - done
  if (done) parts.push(`${done} görev tamamlandı`)
  if (failed) parts.push(`${failed} görev başarısız`)
  return parts.length ? parts.join(', ') : null
}
