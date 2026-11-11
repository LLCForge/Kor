// [g21] Metin belgeleri: DOCX (#174), EPUB okuyucu (#180), Markdown + içindekiler (#178), SVG kod/görünüm (#185).
//      DOCX/EPUB HTML'i sanitizeDocHtml'den (DOMPurify izin listesi) geçer; Markdown mevcut güvenli Markdown
//      bileşeniyle, SVG ise g8'in SvgBlock'u (SandboxedPreview + DOMPurify) ile çizilir.
import { ChevronLeft, ChevronRight, ListTree } from 'lucide-react'
import { useLayoutEffect, useMemo, useRef, useState } from 'react'
import { api } from '../api.ts'
import { Markdown } from '../Markdown.tsx'
import { loadRichBundle } from '../rich-lazy.ts'
import { MAX_RICH_SOURCE } from '../rich-sanitize.ts'
import { SvgBlock } from '../SvgBlock.tsx'
import { CleanHtml, DocBar, Failure, Loading, scrollWithin, useDocQuery, useScrollRatio } from './DocChrome.tsx'
import type { G21DocxApi } from './docx-entry.ts'
import { openEpub, type EpubBook } from './epub.ts'
import { sanitizeDocHtml } from './sanitize.ts'
import { scanMarkup, scanOfficePackage, type ScanFinding } from './scan.ts'
import { sniffImageMime, toBase64 } from './gltf-pack.ts'
import { findEntry, listZip, readZipEntry, readZipText, resolveRelative } from './zip.ts'
import { percentText } from '../ui-locale.ts'

const MAX_EMBED_IMAGE = 10 * 1024 * 1024
const percent = (r: number) => percentText(Math.round(r * 100))

export const loadDocxLib = () => loadRichBundle<G21DocxApi>('./g21-docx.js', '__g21Docx')

/** DOCX'i temizlenmiş HTML'e çevirir; paket taraması (makro, ActiveX, dış ilişki) ile birlikte. */
export async function convertDocx(path: string) {
  const bytes = await api.g21ReadDoc(path)
  const entries = listZip(bytes)
  const rels = await Promise.all(entries.filter((e) => e.name.endsWith('.rels')).slice(0, 200).map((e) => readZipText(bytes, e)))
  const findings = scanOfficePackage(
    entries.map((e) => e.name),
    rels,
  )
  const { html, warnings } = await (await loadDocxLib()).toHtml(bytes.slice().buffer)
  const clean = sanitizeDocHtml(html)
  return { ...clean, findings, warnings, text: clean.fragment.textContent ?? '' }
}

export function DocxViewer({ path }: { path: string }) {
  const { data, error } = useDocQuery(['g21', 'docx', path], () => convertDocx(path))
  const scrollRef = useRef<HTMLDivElement>(null)
  const [ratio, onScroll] = useScrollRatio(scrollRef)
  if (error) return <Failure error={error} />
  if (!data) return <Loading label="Word belgesi dönüştürülüyor..." />
  return (
    <div className="g21-viewer">
      <DocBar position={{ label: percent(ratio), ratio }} sampleText={data.text.slice(0, 20_000)} findings={data.findings} removed={data.removed} quoteFrom={scrollRef} source={path}>
        {data.warnings.length > 0 && (
          <details className="g21-notes">
            <summary>Dönüşüm notları ({data.warnings.length})</summary>
            <ul>
              {data.warnings.slice(0, 50).map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          </details>
        )}
      </DocBar>
      <div className="g21-scroll" ref={scrollRef} onScroll={onScroll} tabIndex={0} aria-label={`${path} içeriği`}>
        <CleanHtml fragment={data.fragment} />
      </div>
    </div>
  )
}

/** EPUB bölümünü okur: görseller arşivden data: URI'ye çevrilir, gövde temizlenir. */
export async function loadEpubChapter(book: EpubBook, index: number) {
  const chapter = book.chapters[index]
  if (!chapter) throw new Error('Bölüm bulunamadı')
  const entry = findEntry(book.entries, chapter.path)
  if (!entry) throw new Error(`Bölüm dosyası yok: ${chapter.path}`)
  const source = await readZipText(book.bytes, entry)
  const findings = scanMarkup(source)
  let doc = new DOMParser().parseFromString(source, 'application/xhtml+xml')
  if (doc.querySelector('parsererror')) doc = new DOMParser().parseFromString(source, 'text/html')
  const toDataUri = async (ref: string | null): Promise<string | null> => {
    if (!ref) return null
    try {
      const imgEntry = findEntry(book.entries, resolveRelative(chapter.path, ref))
      if (!imgEntry || imgEntry.size > MAX_EMBED_IMAGE) return null
      const bytes = await readZipEntry(book.bytes, imgEntry, MAX_EMBED_IMAGE)
      const mime = sniffImageMime(bytes) ?? (/\.gif$/i.test(imgEntry.name) ? 'image/gif' : null)
      return mime ? `data:${mime};base64,${toBase64(bytes)}` : null
    } catch {
      return null // dış ya da geçersiz referans: görsel gösterilmez
    }
  }
  // SVG sarmalı kapak görselleri (<svg><image href>) düz <img>'e çevrilir; SVG etiketleri temizleyicide kapalıdır.
  for (const image of [...doc.getElementsByTagNameNS('http://www.w3.org/2000/svg', 'image')]) {
    const img = doc.createElementNS('http://www.w3.org/1999/xhtml', 'img')
    img.setAttribute('src', image.getAttribute('href') ?? image.getAttributeNS('http://www.w3.org/1999/xlink', 'href') ?? '')
    img.setAttribute('alt', '')
    const svg = image.closest('svg')
    ;(svg ?? image).replaceWith(img)
  }
  for (const img of [...doc.getElementsByTagName('img')]) {
    const uri = await toDataUri(img.getAttribute('src'))
    if (uri) img.setAttribute('src', uri)
    else img.removeAttribute('src')
  }
  const body = doc.body ?? doc.getElementsByTagName('body')[0]
  const clean = sanitizeDocHtml(body?.innerHTML ?? '')
  return { ...clean, findings, text: clean.fragment.textContent ?? '' }
}

export function EpubViewer({ path }: { path: string }) {
  const book = useDocQuery(['g21', 'epub', path], async () => openEpub(await api.g21ReadDoc(path)))
  const [index, setIndex] = useState(0)
  const chapter = useDocQuery(book.data ? ['g21', 'epub-chapter', path, index] : null, () => loadEpubChapter(book.data as EpubBook, index))
  const scrollRef = useRef<HTMLDivElement>(null)
  const [ratio, onScroll] = useScrollRatio(scrollRef)

  useLayoutEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 })
  }, [index])

  if (book.error) return <Failure error={book.error} />
  if (!book.data) return <Loading label="E-kitap açılıyor..." />
  const chapters = book.data.chapters
  const total = chapters.length
  const findings: ScanFinding[] = chapter.data?.findings ?? []
  return (
    <div className="g21-viewer">
      <DocBar
        position={{ label: `Bölüm ${index + 1} / ${total}`, ratio: (index + ratio) / total }}
        sampleText={chapter.data?.text.slice(0, 20_000)}
        findings={findings}
        removed={chapter.data?.removed}
        quoteFrom={scrollRef}
        source={`${path}, ${chapters[index]?.title ?? ''}`}
      >
        <button type="button" className="action-btn" onClick={() => setIndex((i) => Math.max(0, i - 1))} disabled={index === 0} aria-label="Önceki bölüm">
          <ChevronLeft size={15} aria-hidden="true" />
        </button>
        <select className="g21-select" value={index} onChange={(e) => setIndex(Number(e.target.value))} aria-label="Bölüm seç">
          {chapters.map((c, i) => (
            <option key={c.path} value={i}>
              {c.title}
            </option>
          ))}
        </select>
        <button type="button" className="action-btn" onClick={() => setIndex((i) => Math.min(total - 1, i + 1))} disabled={index >= total - 1} aria-label="Sonraki bölüm">
          <ChevronRight size={15} aria-hidden="true" />
        </button>
      </DocBar>
      {book.data.title && <p className="g21-doc-title">{book.data.title}</p>}
      <div className="g21-scroll" ref={scrollRef} onScroll={onScroll} tabIndex={0} aria-label={`${path} bölüm ${index + 1}`}>
        {chapter.error ? <Failure error={chapter.error} /> : chapter.data ? <CleanHtml fragment={chapter.data.fragment} className="g21-epub" /> : <Loading />}
      </div>
    </div>
  )
}

interface TocItem {
  level: number
  text: string
}

export function MarkdownDoc({ path, content }: { path: string; content: string }) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const [ratio, onScroll] = useScrollRatio(scrollRef)
  const [toc, setToc] = useState<TocItem[]>([])
  const [showToc, setShowToc] = useState(true)
  const findings = useMemo(() => scanMarkup(content), [content])

  // İçindekiler çizilmiş başlıklardan çıkarılır (kod bloğu içindeki # satırları zaten başlık olmaz).
  useLayoutEffect(() => {
    const heads = bodyRef.current?.querySelectorAll('h1, h2, h3, h4') ?? []
    setToc([...heads].map((h) => ({ level: Number(h.tagName.slice(1)), text: h.textContent ?? '' })))
  }, [content])

  const jump = (i: number) => scrollWithin(scrollRef.current, bodyRef.current?.querySelectorAll('h1, h2, h3, h4')[i])

  return (
    <div className="g21-viewer">
      <DocBar position={{ label: percent(ratio), ratio }} sampleText={content.slice(0, 20_000)} findings={findings} quoteFrom={scrollRef} source={path}>
        {toc.length > 1 && (
          <button type="button" className={`action-btn${showToc ? ' active' : ''}`} onClick={() => setShowToc((s) => !s)} aria-pressed={showToc} title="İçindekiler">
            <ListTree size={15} aria-hidden="true" />
          </button>
        )}
      </DocBar>
      <div className="g21-md-body">
        {showToc && toc.length > 1 && (
          <nav className="g21-toc" aria-label="İçindekiler">
            <ol>
              {toc.map((item, i) => (
                <li key={i} style={{ paddingInlineStart: (item.level - 1) * 12 }}>
                  <button type="button" onClick={() => jump(i)}>
                    {item.text}
                  </button>
                </li>
              ))}
            </ol>
          </nav>
        )}
        <div className="g21-scroll" ref={scrollRef} onScroll={onScroll} tabIndex={0} aria-label={`${path} içeriği`}>
          <div ref={bodyRef} className="prose g21-md">
            <Markdown text={content} />
          </div>
        </div>
      </div>
    </div>
  )
}

export function SvgDoc({ content, fallbackSrc }: { content: string; fallbackSrc: string }) {
  const findings = useMemo(() => scanMarkup(content), [content])
  return (
    <div className="g21-viewer">
      <DocBar findings={findings} />
      <div className="g21-scroll">
        {content.length <= MAX_RICH_SOURCE ? <SvgBlock code={content} language="svg" /> : <img className="g4-image" src={fallbackSrc} alt="SVG önizlemesi" />}
      </div>
    </div>
  )
}
