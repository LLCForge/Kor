// §27.15 eskimiş not nedenleri (Rust `kor_memory::digest::Reason`).
import type { StaleReason } from '../../../ipc-ek14.ts'

export function reasonText(r: StaleReason): string {
  switch (r.kind) {
    case 'project_missing':
      return `proje klasörü yok (${r.project})`
    case 'file_missing':
      return `${r.file} silinmiş`
    case 'file_changed':
      return `${r.file} ${new Date(r.modified * 1000).toLocaleDateString()} tarihinde değişti`
    case 'symbol_gone':
      return `${r.symbol} artık dosyalarında yok`
  }
}
