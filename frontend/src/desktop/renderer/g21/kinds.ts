// [g21] Dosya uzantısından zengin önizleme türü. Uzantı yalnızca görüntüleyici seçimi içindir; her
//      görüntüleyici içeriği kendi imzası/yapısıyla doğrular (uzantıya güvenilmez).

export type G21Kind = 'pdf' | 'docx' | 'sheet' | 'csv' | 'epub' | 'archive' | 'model' | 'font' | 'image' | 'svg' | 'markdown'

const BY_EXT: Record<string, G21Kind> = {
  pdf: 'pdf',
  docx: 'docx',
  xlsx: 'sheet',
  xlsm: 'sheet',
  csv: 'csv',
  tsv: 'csv',
  epub: 'epub',
  zip: 'archive',
  jar: 'archive',
  apk: 'archive',
  nupkg: 'archive',
  vsix: 'archive',
  whl: 'archive',
  odt: 'archive',
  ods: 'archive',
  pptx: 'archive',
  glb: 'model',
  gltf: 'model',
  ttf: 'font',
  otf: 'font',
  ttc: 'font',
  woff: 'font',
  woff2: 'font',
  png: 'image',
  jpg: 'image',
  jpeg: 'image',
  gif: 'image',
  webp: 'image',
  svg: 'svg',
  md: 'markdown',
  markdown: 'markdown',
  mdx: 'markdown',
}

export function g21KindOf(path: string): G21Kind | null {
  const dot = path.lastIndexOf('.')
  if (dot < 0 || dot < path.lastIndexOf('/')) return null
  return BY_EXT[path.slice(dot + 1).toLowerCase()] ?? null
}

/** Ayrı bayt okuması gerektiren (g4 metin önizlemesinin göstermediği) ikili türler. */
export const BINARY_KINDS: ReadonlySet<G21Kind> = new Set(['docx', 'sheet', 'epub', 'archive', 'model', 'font'])
