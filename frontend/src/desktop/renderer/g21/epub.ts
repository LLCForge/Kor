// [g21] EPUB paket yapısı (#180): container.xml -> OPF (manifest + spine) -> bölüm listesi ve başlıklar (NCX
//      ya da EPUB3 nav). Saf ve DOM'suz; bölüm XHTML'i renderer'da DOMPurify ile temizlenerek gösterilir.
import { collectElements, firstElementText, localName, scanXml } from './xml.ts'
import { findEntry, listZip, readZipText, resolveRelative, type ZipEntryInfo } from './zip.ts'

export const MAX_CHAPTERS = 2_000

export interface EpubChapter {
  /** Arşiv içi yol (ör. OEBPS/chapter1.xhtml) */
  path: string
  title: string
}

export interface EpubBook {
  bytes: Uint8Array
  entries: ZipEntryInfo[]
  title: string | null
  language: string | null
  chapters: EpubChapter[]
}

/** container.xml içinden OPF yolunu bulur. */
export function opfPathFromContainer(xml: string): string | null {
  for (const rootfile of collectElements(xml, 'rootfile')) {
    const path = rootfile['full-path']
    if (path && (!rootfile['media-type'] || rootfile['media-type'] === 'application/oebps-package+xml')) return path
  }
  return null
}

export interface OpfInfo {
  title: string | null
  language: string | null
  /** Okuma sırasındaki bölüm yolları (arşiv içi) */
  spine: string[]
  /** NCX (EPUB2) ya da nav (EPUB3) belgesinin arşiv içi yolu */
  tocPath: string | null
  tocKind: 'ncx' | 'nav' | null
}

const DOC_TYPES = new Set(['application/xhtml+xml', 'text/html', 'application/x-dtbook+xml'])

/** OPF paket belgesini ayrıştırır. */
export function parseOpf(xml: string, opfPath: string): OpfInfo {
  const manifest = new Map<string, { href: string; type: string; props: string }>()
  for (const item of collectElements(xml, 'item')) {
    if (item.id && item.href) manifest.set(item.id, { href: item.href, type: item['media-type'] ?? '', props: item.properties ?? '' })
  }
  const resolve = (href: string): string | null => {
    try {
      return resolveRelative(opfPath, href)
    } catch {
      return null
    }
  }
  const spine: string[] = []
  let ncxId: string | null = null
  scanXml(xml, {
    open: (tag, attrs) => {
      const name = localName(tag)
      if (name === 'spine') ncxId = attrs.toc ?? null
      else if (name === 'itemref' && spine.length < MAX_CHAPTERS) {
        const item = attrs.idref ? manifest.get(attrs.idref) : undefined
        const path = item && DOC_TYPES.has(item.type) ? resolve(item.href) : null
        if (path) spine.push(path)
      }
    },
  })
  const nav = [...manifest.values()].find((i) => i.props.split(/\s+/).includes('nav'))
  const ncx = ncxId ? manifest.get(ncxId) : [...manifest.values()].find((i) => i.type === 'application/x-dtbncx+xml')
  const tocPath = nav ? resolve(nav.href) : ncx ? resolve(ncx.href) : null
  return {
    title: firstElementText(xml, 'title'),
    language: firstElementText(xml, 'language'),
    spine,
    tocPath,
    tocKind: tocPath ? (nav ? 'nav' : 'ncx') : null,
  }
}

/** NCX ya da nav belgesinden "bölüm yolu -> başlık" eşlemesi çıkarır (ilk görülen başlık kazanır). */
export function parseToc(xml: string, tocPath: string, kind: 'ncx' | 'nav'): Map<string, string> {
  const titles = new Map<string, string>()
  const add = (href: string | undefined, label: string) => {
    const text = label.replace(/\s+/g, ' ').trim()
    if (!href || !text) return
    try {
      const path = resolveRelative(tocPath, href)
      if (!titles.has(path)) titles.set(path, text)
    } catch {
      // Dış ya da geçersiz bağlantılar içindekilerden atlanır.
    }
  }
  let label = ''
  let inText = false
  let href: string | undefined
  scanXml(xml, {
    open: (tag, attrs) => {
      const name = localName(tag)
      if (kind === 'ncx') {
        if (name === 'navPoint') label = ''
        else if (name === 'text') inText = true
        else if (name === 'content') add(attrs.src, label)
      } else if (name === 'a') {
        href = attrs.href
        label = ''
        inText = true
      }
    },
    close: (tag) => {
      const name = localName(tag)
      if (kind === 'ncx' && name === 'text') inText = false
      else if (kind === 'nav' && name === 'a') {
        add(href, label)
        inText = false
      }
    },
    text: (t) => {
      if (inText) label += t
    },
  })
  return titles
}

/** EPUB'u açar ve bölüm listesini çıkarır. */
export async function openEpub(bytes: Uint8Array): Promise<EpubBook> {
  const entries = listZip(bytes)
  const container = findEntry(entries, 'META-INF/container.xml')
  if (!container) throw new Error('Geçerli bir EPUB değil (META-INF/container.xml yok)')
  const opfPath = opfPathFromContainer(await readZipText(bytes, container))
  const opfEntry = opfPath ? findEntry(entries, opfPath) : null
  if (!opfPath || !opfEntry) throw new Error('EPUB paket belgesi (OPF) bulunamadı')
  const opf = parseOpf(await readZipText(bytes, opfEntry), opfPath)
  let titles = new Map<string, string>()
  const tocEntry = opf.tocPath ? findEntry(entries, opf.tocPath) : null
  if (tocEntry && opf.tocPath && opf.tocKind) {
    try {
      titles = parseToc(await readZipText(bytes, tocEntry), opf.tocPath, opf.tocKind)
    } catch (err) {
      console.warn('EPUB içindekiler okunamadı:', err)
    }
  }
  const chapters = opf.spine
    .filter((path) => findEntry(entries, path) !== null)
    .map((path, i) => ({ path, title: titles.get(path) ?? `Bölüm ${i + 1}` }))
  if (chapters.length === 0) throw new Error('EPUB içinde okunabilir bölüm bulunamadı')
  return { bytes, entries, title: opf.title, language: opf.language, chapters }
}
