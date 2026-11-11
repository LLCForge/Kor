// [g21] PDF sayfa görüntüleyici (#171 sayfa, #172 metin katmanı + arama, #173 küçük resim şeridi, #186 konum,
//      #189 yalnız görünen sayfaları çizme, #187 alıntı, #188 dil, #190 tarama).
//      Güvenlik: pdf.js yalnızca ayrı çalışanda ayrıştırır; betik motoru (pdf.sandbox) ve bağlantı/form katmanı
//      (AnnotationLayer) hiç yüklenmez; XFA kapalı; CMap/standart yazı tipi URL'si verilmez (ağdan hiçbir şey
//      çekilmez). Metin katmanı pdf.js TextLayer ile metin düğümü olarak çizilir (HTML enjeksiyonu yok).
import { ChevronDown, ChevronUp, Columns2, Minus, Plus, Search } from 'lucide-react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent, type ReactNode, type RefObject } from 'react'
import useSWR from 'swr'
import type { PDFDocumentProxy } from 'pdfjs-dist'
import { api } from '../api.ts'
import { loadRichBundle } from '../rich-lazy.ts'
import { DocBar, Failure, Loading, scrollWithin, useDocQuery } from './DocChrome.tsx'
import type { G21PdfApi } from './pdf-entry.ts'
import { scanPdf } from './scan.ts'
import { percentText } from '../ui-locale.ts'

const MAX_SEARCH_PAGES = 2_000
const MIN_SCALE = 0.25
const MAX_SCALE = 4
const THUMB_WIDTH = 96

const loadPdfLib = () => loadRichBundle<G21PdfApi>('./g21-pdf.js', '__g21Pdf')

/** Kök içinde [data-page] öğelerinden hangilerinin (ön yükleme payıyla) görünür olduğunu izler. */
function useVisiblePages(rootRef: RefObject<HTMLElement | null>, deps: readonly unknown[], margin: string): ReadonlySet<number> {
  const [visible, setVisible] = useState<ReadonlySet<number>>(() => new Set())
  useLayoutEffect(() => {
    const root = rootRef.current
    if (!root) return
    const io = new IntersectionObserver(
      (entries) =>
        setVisible((prev) => {
          const next = new Set(prev)
          for (const e of entries) {
            const n = Number((e.target as HTMLElement).dataset.page)
            if (e.isIntersecting) next.add(n)
            else next.delete(n)
          }
          return next
        }),
      { root, rootMargin: margin },
    )
    for (const el of root.querySelectorAll<HTMLElement>('[data-page]')) io.observe(el)
    return () => io.disconnect()
  }, deps) // gözlemci yalnızca sayfa düzeni (deps) değişince yeniden kurulur
  return visible
}

function highlight(container: HTMLElement | null, query: string): void {
  if (!container) return
  const q = query.trim().toLocaleLowerCase('tr')
  for (const span of container.querySelectorAll('span')) {
    span.classList.toggle('g21-hit', q.length >= 2 && (span.textContent ?? '').toLocaleLowerCase('tr').includes(q))
  }
}

interface PageProps {
  doc: PDFDocumentProxy
  lib: G21PdfApi
  pageNumber: number
  /** Yer tutucu kutu (ilk sayfanın ölçeklenmiş boyutu); farklı boyutlu sayfa kutuya sığdırılır */
  width: number
  height: number
  visible: boolean
  query: string
}

function PdfPage({ doc, lib, pageNumber, width, height, visible, query }: PageProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const textRef = useRef<HTMLDivElement>(null)
  const [error, setError] = useState<string | null>(null)
  const queryRef = useRef(query)
  queryRef.current = query

  useEffect(() => {
    if (!visible) return
    let cancelled = false
    let renderTask: ReturnType<Awaited<ReturnType<PDFDocumentProxy['getPage']>>['render']> | null = null
    let textLayer: InstanceType<G21PdfApi['TextLayer']> | null = null
    let page: Awaited<ReturnType<PDFDocumentProxy['getPage']>> | null = null
    void (async () => {
      page = await doc.getPage(pageNumber)
      const canvas = canvasRef.current
      const textDiv = textRef.current
      if (cancelled || !canvas || !textDiv) return
      const unit = page.getViewport({ scale: 1 })
      const viewport = page.getViewport({ scale: Math.min(width / unit.width, height / unit.height) })
      const ratio = Math.min(window.devicePixelRatio || 1, 2)
      canvas.width = Math.floor(viewport.width * ratio)
      canvas.height = Math.floor(viewport.height * ratio)
      canvas.style.width = `${Math.floor(viewport.width)}px`
      canvas.style.height = `${Math.floor(viewport.height)}px`
      renderTask = page.render({ canvas, viewport, ...(ratio !== 1 ? { transform: [ratio, 0, 0, ratio, 0, 0] } : {}) })
      await renderTask.promise
      if (cancelled) return
      textDiv.replaceChildren()
      textDiv.style.setProperty('--scale-factor', String(viewport.scale))
      textDiv.style.setProperty('--user-unit', String(viewport.userUnit))
      textLayer = new lib.TextLayer({ textContentSource: page.streamTextContent(), container: textDiv, viewport })
      await textLayer.render()
      if (!cancelled) highlight(textDiv, queryRef.current)
    })().catch((err: unknown) => {
      if (cancelled || (err instanceof Error && err.name === 'RenderingCancelledException')) return
      console.error(`PDF sayfa ${pageNumber} çizilemedi:`, err)
      setError('Sayfa çizilemedi.')
    })
    return () => {
      cancelled = true
      renderTask?.cancel()
      textLayer?.cancel()
      page?.cleanup()
    }
  }, [visible, doc, lib, pageNumber, width, height])

  useEffect(() => highlight(textRef.current, query), [query])

  return (
    <div className="g21-pdf-page" data-page={pageNumber} style={{ width, height }}>
      {visible && (
        <>
          <canvas ref={canvasRef} aria-hidden="true" />
          <div ref={textRef} className="textLayer" />
        </>
      )}
      {error && <p className="error-text g21-page-error">{error}</p>}
    </div>
  )
}

function PdfThumb({ doc, pageNumber, visible, active, onSelect }: { doc: PDFDocumentProxy; pageNumber: number; visible: boolean; active: boolean; onSelect: () => void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    if (!visible) return
    let cancelled = false
    let task: { cancel: () => void } | null = null
    void (async () => {
      const page = await doc.getPage(pageNumber)
      const canvas = canvasRef.current
      if (cancelled || !canvas) return
      const base = page.getViewport({ scale: 1 })
      const viewport = page.getViewport({ scale: THUMB_WIDTH / base.width })
      canvas.width = Math.floor(viewport.width)
      canvas.height = Math.floor(viewport.height)
      const render = page.render({ canvas, viewport })
      task = render
      await render.promise
    })().catch((err: unknown) => {
      if (!(err instanceof Error && err.name === 'RenderingCancelledException')) console.warn('Küçük resim çizilemedi:', err)
    })
    return () => {
      cancelled = true
      task?.cancel()
    }
  }, [visible, doc, pageNumber])
  return (
    <button type="button" className={`g21-thumb${active ? ' active' : ''}`} data-page={pageNumber} onClick={onSelect} aria-label={`Sayfa ${pageNumber}`} aria-current={active ? 'page' : undefined}>
      {visible ? <canvas ref={canvasRef} aria-hidden="true" /> : <span className="g21-thumb-ph" />}
      <span>{pageNumber}</span>
    </button>
  )
}

async function pageText(doc: PDFDocumentProxy, n: number): Promise<string> {
  const content = await (await doc.getPage(n)).getTextContent()
  return content.items.map((item) => ('str' in item ? item.str + (item.hasEOL ? '\n' : ' ') : '')).join('')
}

interface ViewerProps {
  path: string
  /** pdf.js kullanılamazsa gösterilecek düz metin görünümü */
  fallback: ReactNode
}

export function PdfViewer({ path, fallback }: ViewerProps) {
  const lib = useSWR('g21:lib:pdf', loadPdfLib, { revalidateOnFocus: false, shouldRetryOnError: false })
  const bytes = useDocQuery(['g21', 'bytes', path], () => api.g21ReadDoc(path))
  const findings = useMemo(() => (bytes.data ? scanPdf(bytes.data) : undefined), [bytes.data])
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    const pdf = lib.data
    if (!pdf || !bytes.data) return
    setLoadError(null)
    let cancelled = false
    let worker: ReturnType<G21PdfApi['createWorker']> | null = null
    try {
      worker = pdf.createWorker()
    } catch (err) {
      console.error('pdf.js çalışanı başlatılamadı:', err)
      setLoadError('PDF çalışanı başlatılamadı; düz metin görünümü gösteriliyor.')
      return
    }
    // pdf.js veriyi çalışana aktarır (ArrayBuffer devredilir); önbellekteki kopya bozulmasın diye kopyalanır.
    const task = pdf.getDocument({ data: bytes.data.slice(), worker: worker.pdfWorker, enableXfa: false, disableAutoFetch: true, disableStream: true, stopAtErrors: false, verbosity: 0 })
    task.promise.then(
      (d) => {
        if (!cancelled) setDoc(d)
      },
      (err: unknown) => {
        if (cancelled) return
        console.error('PDF açılamadı:', err)
        setLoadError(err instanceof Error && err.name === 'PasswordException' ? 'Parola korumalı PDF önizlenemez.' : 'PDF açılamadı (bozuk veya desteklenmeyen dosya).')
      },
    )
    return () => {
      cancelled = true
      setDoc(null)
      void task.destroy().finally(() => worker?.terminate())
    }
  }, [lib.data, bytes.data])

  if (lib.error || bytes.error || loadError) {
    return (
      <div className="g21-fallback">
        {lib.error ? <Failure error={lib.error} /> : bytes.error ? <Failure error={bytes.error} /> : <p className="error-text pad">{loadError}</p>}
        {fallback}
      </div>
    )
  }
  if (!doc || !lib.data) return <Loading label="PDF hazırlanıyor..." />
  return <PdfDocument key={path} doc={doc} lib={lib.data} path={path} findings={findings} />
}

function PdfDocument({ doc, lib, path, findings }: { doc: PDFDocumentProxy; lib: G21PdfApi; path: string; findings: ReturnType<typeof scanPdf> | undefined }) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const thumbsRef = useRef<HTMLDivElement>(null)
  const quoteRef = useRef<HTMLDivElement>(null)
  const total = doc.numPages
  const base = useSWR(['g21:pdf:base', doc.fingerprints[0], path], async () => {
    const vp = (await doc.getPage(1)).getViewport({ scale: 1 })
    return { width: vp.width, height: vp.height }
  })
  const sample = useSWR(['g21:pdf:sample', doc.fingerprints[0], path], async () => {
    const texts: string[] = []
    for (let n = 1; n <= Math.min(total, 3); n++) texts.push(await pageText(doc, n))
    return texts.join('\n')
  })
  const [fit, setFit] = useState(true)
  const [scale, setScale] = useState(1)
  const [current, setCurrent] = useState(1)
  const [pageInput, setPageInput] = useState('1')
  const [thumbs, setThumbs] = useState(total > 1)
  const [query, setQuery] = useState('')
  const [applied, setApplied] = useState('')
  const [hits, setHits] = useState<number[] | null>(null)
  const [searching, setSearching] = useState(false)
  const textCache = useRef(new Map<number, string>())
  const searchToken = useRef(0)

  const width = base.data ? Math.floor(base.data.width * scale) : 0
  const height = base.data ? Math.floor(base.data.height * scale) : 0
  const visible = useVisiblePages(scrollRef, [total, width, height], '150% 0px')
  const visibleThumbs = useVisiblePages(thumbsRef, [total, thumbs, base.data], '300% 0px')

  // Genişliğe sığdırma: kap boyutu değiştikçe ölçek yeniden hesaplanır.
  useLayoutEffect(() => {
    const el = scrollRef.current
    if (!el || !fit || !base.data) return
    const baseWidth = base.data.width
    const update = () => setScale(Math.max(MIN_SCALE, Math.min(MAX_SCALE, (el.clientWidth - 32) / baseWidth)))
    update()
    const ro = new ResizeObserver(update)
    ro.observe(el)
    return () => ro.disconnect()
  }, [fit, base.data])

  const pageElement = (n: number) => scrollRef.current?.querySelector<HTMLElement>(`[data-page="${n}"]`) ?? null
  const goTo = (n: number) => {
    const target = Math.max(1, Math.min(total, n))
    scrollWithin(scrollRef.current, pageElement(target), 16)
    setCurrent(target)
    setPageInput(String(target))
  }

  const frame = useRef(0)
  const onScroll = () => {
    if (frame.current) return
    frame.current = requestAnimationFrame(() => {
      frame.current = 0
      const el = scrollRef.current
      if (!el || height === 0) return
      // Sayfalar eşit aralıklı yerleşir (yükseklik + 16px boşluk); görünümün üçte biri hizasındaki sayfa güncel sayılır.
      const n = Math.max(1, Math.min(total, Math.floor((el.scrollTop + el.clientHeight / 3) / (height + 16)) + 1))
      setCurrent(n)
      setPageInput(String(n))
    })
  }
  useEffect(
    () => () => {
      cancelAnimationFrame(frame.current)
      frame.current = 0
    },
    [],
  )

  const zoom = (factor: number) => {
    setFit(false)
    setScale((s) => Math.max(MIN_SCALE, Math.min(MAX_SCALE, Number((s * factor).toFixed(2)))))
  }

  const runSearch = async (e: FormEvent) => {
    e.preventDefault()
    const q = query.trim().toLocaleLowerCase('tr')
    setApplied(query.trim())
    if (q.length < 2) {
      setHits(null)
      return
    }
    const token = ++searchToken.current
    setSearching(true)
    const found: number[] = []
    try {
      for (let n = 1; n <= Math.min(total, MAX_SEARCH_PAGES); n++) {
        let text = textCache.current.get(n)
        if (text === undefined) {
          text = (await pageText(doc, n)).toLocaleLowerCase('tr')
          textCache.current.set(n, text)
        }
        if (token !== searchToken.current) return
        if (text.includes(q)) found.push(n)
      }
      setHits(found)
      if (found[0] !== undefined) goTo(found.find((n) => n >= current) ?? found[0])
    } catch (err) {
      console.error('PDF araması başarısız:', err)
      setHits([])
    } finally {
      if (token === searchToken.current) setSearching(false)
    }
  }

  const stepHit = (dir: 1 | -1) => {
    if (!hits || hits.length === 0) return
    const next = dir === 1 ? (hits.find((n) => n > current) ?? hits[0]) : ([...hits].reverse().find((n) => n < current) ?? hits[hits.length - 1])
    if (next !== undefined) goTo(next)
  }

  if (base.error) return <Failure error={base.error} />
  if (!base.data) return <Loading label="PDF hazırlanıyor..." />
  const pages = Array.from({ length: total }, (_, i) => i + 1)

  return (
    <div className="g21-viewer">
      <DocBar
        position={{ label: `Sayfa ${current} / ${total}`, ratio: total > 1 ? (current - 1) / (total - 1) : 1 }}
        sampleText={sample.data}
        findings={findings}
        quoteFrom={quoteRef}
        source={() => `${path}, s. ${current}`}
      >
        <form
          className="g21-pagejump"
          onSubmit={(e) => {
            e.preventDefault()
            goTo(Number(pageInput) || 1)
          }}
        >
          <input value={pageInput} onChange={(e) => setPageInput(e.target.value.replace(/\D/g, ''))} inputMode="numeric" aria-label="Sayfaya git" />
        </form>
        <button type="button" className="action-btn" onClick={() => setThumbs((t) => !t)} aria-pressed={thumbs} title="Küçük resim şeridi">
          <Columns2 size={15} aria-hidden="true" />
        </button>
        <button type="button" className="action-btn" onClick={() => zoom(1 / 1.2)} title="Uzaklaştır" aria-label="Uzaklaştır">
          <Minus size={15} aria-hidden="true" />
        </button>
        <span className="g21-zoom">{percentText(Math.round(scale * 100))}</span>
        <button type="button" className="action-btn" onClick={() => zoom(1.2)} title="Yakınlaştır" aria-label="Yakınlaştır">
          <Plus size={15} aria-hidden="true" />
        </button>
        <button type="button" className={`action-btn${fit ? ' active' : ''}`} onClick={() => setFit(true)} aria-pressed={fit} title="Genişliğe sığdır">
          Sığdır
        </button>
      </DocBar>
      <form className="g21-search" onSubmit={(e) => void runSearch(e)} role="search">
        <Search size={14} aria-hidden="true" />
        <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="PDF içinde ara (Enter)" aria-label="PDF içinde ara" />
        {searching && <span className="g21-muted">aranıyor…</span>}
        {!searching && hits && (
          <span className="g21-muted" role="status">
            {hits.length === 0 ? 'eşleşme yok' : `${hits.length} sayfada bulundu`}
          </span>
        )}
        <button type="button" className="action-btn" onClick={() => stepHit(-1)} disabled={!hits?.length} aria-label="Önceki eşleşme">
          <ChevronUp size={15} aria-hidden="true" />
        </button>
        <button type="button" className="action-btn" onClick={() => stepHit(1)} disabled={!hits?.length} aria-label="Sonraki eşleşme">
          <ChevronDown size={15} aria-hidden="true" />
        </button>
      </form>
      <div className="g21-pdf-body">
        {thumbs && (
          <nav className="g21-thumbs" ref={thumbsRef} aria-label="Sayfa küçük resimleri">
            {pages.map((n) => (
              <PdfThumb key={n} doc={doc} pageNumber={n} visible={visibleThumbs.has(n)} active={n === current} onSelect={() => goTo(n)} />
            ))}
          </nav>
        )}
        <div className="g21-pdf-scroll" ref={scrollRef} onScroll={onScroll} tabIndex={0} aria-label={`${path} sayfaları`}>
          <div ref={quoteRef} className="g21-pdf-pages">
            {pages.map((n) => (
              <PdfPage key={n} doc={doc} lib={lib} pageNumber={n} width={width} height={height} visible={visible.has(n)} query={applied} />
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
