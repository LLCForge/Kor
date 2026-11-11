// [g21] Belge ve zengin dosya önizlemesi (OZELLIK-ONERILERI-2 #171-190) için FilePreview'e tek bağlantı noktası.
//      g4'ün önizleme sonucuna (metin/ikili/çok büyük) ve dosya türüne göre uygun görüntüleyiciyi seçer; türü
//      tanınmayan dosyada null döner ve g4'ün kendi görünümü kullanılır.
import type { ReactNode } from 'react'
import type { FilePreview as Preview } from '../../ipc-g4.ts'
import { DocxViewer, EpubViewer, MarkdownDoc, SvgDoc } from './DocViewers.tsx'
import { BINARY_KINDS, g21KindOf, type G21Kind } from './kinds.ts'
import { ArchiveViewer, FontViewer, ImageMetaPanel, ModelViewer } from './MediaViewers.tsx'
import { PdfViewer } from './PdfViewer.tsx'
import { CsvViewer, XlsxViewer } from './SheetViewer.tsx'

export { ComparePanel } from './ComparePanel.tsx'
export { ImageMetaPanel as G21ImageMeta, PdfViewer as G21Pdf }

/** Metin olarak gelen ama "Görünüm" kipinde zengin çizilen ek türler (CSV/TSV tablo, .gltf model). */
export function g21CanRender(path: string): boolean {
  const kind = g21KindOf(path)
  return kind === 'csv' || kind === 'model'
}

/** Karşılaştırma (kelime farkı) yalnızca metni çıkarılabilen dosyalarda sunulur. */
export function g21CanCompare(path: string, preview: Preview['kind']): boolean {
  const kind = g21KindOf(path)
  return kind === 'pdf' || kind === 'docx' || kind === 'epub' || (preview === 'text' && kind !== 'svg' && kind !== 'model')
}

function BinaryDoc({ kind, path }: { kind: G21Kind; path: string }) {
  switch (kind) {
    case 'docx':
      return <DocxViewer path={path} />
    case 'sheet':
      return <XlsxViewer path={path} />
    case 'epub':
      return <EpubViewer path={path} />
    case 'archive':
      return <ArchiveViewer path={path} />
    case 'model':
      return <ModelViewer path={path} />
    case 'font':
      return <FontViewer path={path} />
    default:
      return null
  }
}

interface BodyArgs {
  path: string
  data: Preview
  mode: 'render' | 'source'
  editing: boolean
}

/** g21 görüntüleyicisi varsa onu, yoksa null döndürür (çağıran g4 görünümüne düşer). */
export function g21Body({ path, data, mode, editing }: BodyArgs): ReactNode | null {
  const kind = g21KindOf(path)
  if (!kind || editing) return null
  if (data.kind === 'binary' || data.kind === 'tooLarge') {
    if (kind === 'pdf' && data.kind === 'tooLarge') return <PdfViewer key={path} path={path} fallback={null} />
    return BINARY_KINDS.has(kind) ? <BinaryDoc key={path} kind={kind} path={path} /> : null
  }
  if (data.kind !== 'text' || mode !== 'render') return null
  switch (kind) {
    case 'markdown':
      return <MarkdownDoc path={path} content={data.content} />
    case 'csv':
      return <CsvViewer path={path} content={data.content} />
    case 'model':
      return <ModelViewer key={path} path={path} />
    case 'svg':
      return data.imageDataUrl ? <SvgDoc content={data.content} fallbackSrc={data.imageDataUrl} /> : null
    default:
      return null
  }
}
