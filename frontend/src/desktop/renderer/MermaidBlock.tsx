// [g3] ```mermaid blokları: diyagram ayrı paketten (mermaid.js) tembel yüklenir ve gölge köke çizilir.
//      Tüm Mermaid türlerini (flowchart, sequence, er, state, class, gantt, timeline, journey, gitGraph, pie,
//      quadrant, mindmap, sankey, C4, block, xychart, packet, requirement) genel mermaid.render yolu çizer.
//      Kart UX'i: kaynak/çizim geçişi (30), tema seçimi (24), düğüm arama+vurgu (26), konum haritası (27),
//      satır numaralı hata rozeti (28), subgraph katlama (23).
import { Code, Map, Search, Workflow } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { G17DiagramExport } from './G17Blocks.tsx' // [g17]
import { collapseByTitles, findSubgraphs, parseDiagramError } from './diagram-utils.ts'
import { DiagramMinimap } from './DiagramMinimap.tsx'
import { loadRichBundle } from './rich-lazy.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { foldForSearch } from './search-utils.ts'

type MermaidApi = NonNullable<Window['__g3Mermaid']>

/** Akış sürerken her parçada yeniden çizmemek için kod durulunca çizilir. */
const SETTLE_MS = 400

/** feature 24: hazır Mermaid temaları; 'auto' sistem açık/koyu moduna bağlanır. Seçim cihazda saklanır. */
const THEMES: ReadonlyArray<{ id: string; label: string }> = [
  { id: 'auto', label: 'Otomatik' },
  { id: 'neutral', label: 'Nötr' },
  { id: 'default', label: 'Varsayılan' },
  { id: 'forest', label: 'Orman' },
  { id: 'dark', label: 'Koyu' },
]
const THEME_KEY = 'g3-mermaid-theme'

function initialTheme(): string {
  try {
    const saved = localStorage.getItem(THEME_KEY)
    return saved && THEMES.some((t) => t.id === saved) ? saved : 'auto'
  } catch {
    return 'auto'
  }
}

/** Arama vurgusu için gölge köke eklenen (CSSOM; CSP satır içi stil denetimine tabi değil) ortak biçem sayfası. */
let highlightSheet: CSSStyleSheet | null = null
function ensureHighlightSheet(root: ShadowRoot): void {
  if (!highlightSheet) {
    highlightSheet = new CSSStyleSheet()
    highlightSheet.replaceSync(
      '.g3-dgm-hit{fill:var(--accent)!important;font-weight:700!important}' +
        '.g3-dgm-hit rect,.g3-dgm-hit circle,.g3-dgm-hit polygon,.g3-dgm-hit ellipse,.g3-dgm-hit path{stroke:var(--accent)!important;stroke-width:calc(var(--kor-space-1) / 2)!important}',
    )
  }
  if (!root.adoptedStyleSheets.includes(highlightSheet)) {
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, highlightSheet]
  }
}

/** Gölge kökteki etiket metinlerinde arar; eşleşenleri işaretler, ilkini görünüme kaydırır. Eşleşme sayısını döndürür. */
function applySearch(root: ShadowRoot, query: string): number {
  for (const el of root.querySelectorAll('.g3-dgm-hit')) el.classList.remove('g3-dgm-hit')
  const needle = foldForSearch(query.trim())
  if (!needle) return 0
  let count = 0
  let first: Element | null = null
  for (const el of root.querySelectorAll('text, .nodeLabel, .edgeLabel, .label')) {
    if (foldForSearch(el.textContent ?? '').includes(needle)) {
      el.classList.add('g3-dgm-hit')
      count++
      if (!first) first = el
    }
  }
  first?.scrollIntoView({ block: 'center', inline: 'center' })
  return count
}

/** Kaynağı satırlara böler; hata satırını vurgular (feature 28). */
function sourceLines(code: string, errorLine: number | null): ReactNode {
  return code.split('\n').map((line, i) => (
    <span key={i} className={errorLine === i + 1 ? 'g3-dgm-errline' : undefined}>
      {line || ' '}
      {'\n'}
    </span>
  ))
}

export function MermaidBlock({ code }: RichBlockProps) {
  const hostRef = useRef<HTMLDivElement>(null)
  const [status, setStatus] = useState<'loading' | 'ok' | 'error'>('loading')
  const [error, setError] = useState<{ line: number | null; message: string } | null>(null)
  const [showSource, setShowSource] = useState(false)
  const [theme, setTheme] = useState(initialTheme)
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set())
  const [query, setQuery] = useState('')
  const [matches, setMatches] = useState<number | null>(null)
  const [overflow, setOverflow] = useState(false)
  const [showMinimap, setShowMinimap] = useState(false)
  const themeId = useId()

  const subgraphs = useMemo(() => findSubgraphs(code), [code])
  const effectiveCode = useMemo(() => collapseByTitles(code, collapsed), [code, collapsed])

  useEffect(() => {
    let cancelled = false
    setStatus('loading')
    const timer = setTimeout(() => {
      const host = hostRef.current
      if (!host) return
      loadRichBundle<MermaidApi>('./mermaid.js', '__g3Mermaid')
        .then((m) => m.render(effectiveCode, host, { theme: theme === 'auto' ? undefined : theme }))
        .then(
          () => {
            if (cancelled) return
            setStatus('ok')
            setOverflow(host.scrollHeight > host.clientHeight + 2 || host.scrollWidth > host.clientWidth + 2)
          },
          (err: unknown) => {
            console.warn('Mermaid çizimi başarısız:', err)
            if (cancelled) return
            setStatus('error')
            setError(parseDiagramError(err instanceof Error ? err.message : String(err)))
          },
        )
    }, SETTLE_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [effectiveCode, theme])

  // Arama vurgusu (feature 26): çizim bitince ve sorgu değişince uygulanır.
  useEffect(() => {
    const root = hostRef.current?.shadowRoot
    if (status !== 'ok' || !root) {
      setMatches(query.trim() ? 0 : null)
      return
    }
    ensureHighlightSheet(root)
    setMatches(query.trim() ? applySearch(root, query) : null)
  }, [query, status, effectiveCode])

  const pickTheme = (id: string) => {
    setTheme(id)
    try {
      localStorage.setItem(THEME_KEY, id)
    } catch {
      // yerel saklama yoksa yoksay (gizli pencere vb.)
    }
  }

  const toggleCollapse = (title: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(title)) next.delete(title)
      else next.add(title)
      return next
    })
  }

  // Sürükleyerek kaydırma (pan): büyük diyagram host içinde fareyle sağa/sola/yukarı/aşağı taşınır. Tekerlek de çalışır.
  const drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null)
  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    const host = hostRef.current
    if (!host || !overflow || e.button !== 0) return
    drag.current = { x: e.clientX, y: e.clientY, left: host.scrollLeft, top: host.scrollTop }
    host.setPointerCapture(e.pointerId)
    host.style.cursor = 'grabbing'
  }
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const host = hostRef.current
    const d = drag.current
    if (!host || !d) return
    host.scrollLeft = d.left - (e.clientX - d.x)
    host.scrollTop = d.top - (e.clientY - d.y)
  }
  const endDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    const host = hostRef.current
    if (host && drag.current) {
      host.releasePointerCapture(e.pointerId)
      host.style.cursor = ''
    }
    drag.current = null
  }

  return (
    <div className="code-block g3-mermaid">
      <div className="code-head">
        <span className="g3-code-lang">
          <Workflow size={13} aria-hidden="true" /> mermaid
        </span>
        <div className="g3-code-tools">
          {status === 'ok' && (
            <div className="g3-dgm-search">
              <Search size={13} aria-hidden="true" />
              <input
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Düğüm ara"
                aria-label="Diyagramda düğüm ara"
              />
              {matches !== null && <span className="g3-dgm-count">{matches}</span>}
            </div>
          )}
          <label className="sr-only" htmlFor={themeId}>
            Diyagram teması
          </label>
          <select
            id={themeId}
            className="g3-dgm-theme"
            value={theme}
            onChange={(e) => pickTheme(e.target.value)}
            title="Diyagram teması"
          >
            {THEMES.map((t) => (
              <option key={t.id} value={t.id}>
                {t.label}
              </option>
            ))}
          </select>
          {overflow && (
            <button
              type="button"
              className={`code-copy${showMinimap ? ' g3-on' : ''}`}
              onClick={() => setShowMinimap((s) => !s)}
              aria-pressed={showMinimap}
              aria-label="Konum haritası"
              title="Konum haritası"
            >
              <Map size={15} aria-hidden="true" />
            </button>
          )}
          <button
            type="button"
            className={`code-copy${showSource ? ' g3-on' : ''}`}
            onClick={() => setShowSource((s) => !s)}
            aria-pressed={showSource}
            aria-label="Kaynak kodu göster"
            title="Kaynak kodu göster"
          >
            <Code size={15} aria-hidden="true" />
          </button>
          <CopyButton className="code-copy" label="Mermaid kodunu kopyala" getText={() => code} />
          {/* [g17] diyagram dışa aktarma (PNG/SVG/pano/kaynak) */}
          {status === 'ok' && <G17DiagramExport hostRef={hostRef} source={code} />}
        </div>
      </div>

      {subgraphs.length > 0 && (
        <div className="g3-dgm-groups" role="group" aria-label="Alt grafikler">
          {subgraphs.map((s) => (
            <button
              key={`${s.title}-${s.start}`}
              type="button"
              className={`g3-dgm-chip${collapsed.has(s.title) ? ' g3-on' : ''}`}
              onClick={() => toggleCollapse(s.title)}
              aria-pressed={collapsed.has(s.title)}
              title={collapsed.has(s.title) ? `${s.title}: genişlet` : `${s.title}: katla`}
            >
              {collapsed.has(s.title) ? '▸' : '▾'} {s.title}
            </button>
          ))}
        </div>
      )}

      {status === 'loading' && <p className="g3-mermaid-note">Diyagram çiziliyor…</p>}
      {status === 'error' && (
        <p className="g3-mermaid-note error-text" role="alert">
          Diyagram çizilemedi{error?.line ? ` (satır ${error.line})` : ''}
          {error?.message ? `: ${error.message}` : ''}
        </p>
      )}

      <div className="g3-dgm-stage">
        <div
          ref={hostRef}
          className={`g3-mermaid-host${overflow ? ' g3-pan' : ''}`}
          role="img"
          aria-label="Mermaid diyagramı"
          hidden={status !== 'ok'}
          tabIndex={status === 'ok' && overflow ? 0 : undefined}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        />
        {status === 'ok' && showMinimap && overflow && hostRef.current && <DiagramMinimap host={hostRef.current} />}
      </div>

      {(showSource || status === 'error') && <pre className="g3-mermaid-source">{sourceLines(effectiveCode, error?.line ?? null)}</pre>}
    </div>
  )
}

registerRichBlock('mermaid', MermaidBlock)
