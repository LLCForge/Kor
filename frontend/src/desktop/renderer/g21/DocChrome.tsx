// [g21] Belge önizlemelerinin ortak parçaları: veri kancası (SWR, bileşen kapanınca önbellek boşaltılır),
//      araç çubuğu (konum/ilerleme #186, dil #188, güvenlik taraması #190, alıntı #187) ve temizlenmiş HTML gövdesi.
import { Languages, Quote, ShieldAlert, ShieldCheck } from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react'
import useSWR, { unstable_serialize, useSWRConfig } from 'swr'
import { errorMessage } from '../api.ts'
import { detectLanguage } from './lang.ts'
import { quoteToDraft } from './quote.ts'
import { summarizeFindings, type ScanFinding } from './scan.ts'

/** Tek seferlik belge verisi; bileşen kapanınca büyük baytlar SWR önbelleğinde tutulmaz. */
export function useDocQuery<T>(key: readonly unknown[] | null, fetcher: () => Promise<T>) {
  const { cache } = useSWRConfig()
  const swr = useSWR(key, fetcher, { revalidateOnFocus: false, revalidateIfStale: false, revalidateOnReconnect: false, shouldRetryOnError: false })
  const serialized = key ? unstable_serialize(key) : null
  useEffect(
    () => () => {
      if (serialized) cache.delete(serialized)
    },
    [cache, serialized],
  )
  return swr
}

/** Öğeyi yalnızca verilen kap içinde başa kaydırır (scrollIntoView üst panelleri de kaydırırdı). */
export function scrollWithin(container: HTMLElement | null, target: Element | null | undefined, offset = 8): void {
  if (!container || !target) return
  container.scrollTop += target.getBoundingClientRect().top - container.getBoundingClientRect().top - offset
}

/** Kaydırma kabının konumunu 0-1 arası oran olarak izler (rAF ile seyreltilmiş). */
export function useScrollRatio(ref: RefObject<HTMLElement | null>): [number, () => void] {
  const [ratio, setRatio] = useState(0)
  const frame = useRef(0)
  const onScroll = useCallback(() => {
    if (frame.current) return
    frame.current = requestAnimationFrame(() => {
      frame.current = 0
      const el = ref.current
      if (!el) return
      const max = el.scrollHeight - el.clientHeight
      setRatio(max > 0 ? Math.min(1, el.scrollTop / max) : 1)
    })
  }, [ref])
  useEffect(
    () => () => {
      cancelAnimationFrame(frame.current)
      frame.current = 0
    },
    [],
  )
  return [ratio, onScroll]
}

function ScanBadge({ findings, removed }: { findings: readonly ScanFinding[]; removed?: number }) {
  const warn = findings.some((f) => f.severity === 'warn')
  const clean = findings.length === 0 && !removed
  const summary = summarizeFindings(findings)
  return (
    <details className={`g21-scan${warn ? ' g21-scan-warn' : ''}`}>
      <summary title={summary}>
        {clean ? <ShieldCheck size={13} aria-hidden="true" /> : <ShieldAlert size={13} aria-hidden="true" />}
        {clean ? 'Tarama temiz' : `${findings.reduce((s, f) => s + f.count, 0) + (removed ?? 0)} öğe etkisiz`}
      </summary>
      <div className="g21-scan-body" role="note">
        <p>Önizleme öncesi güvenlik taraması. Betikler çalıştırılmaz, dış kaynaklar yüklenmez (ağ kapalı).</p>
        {findings.length > 0 && (
          <ul>
            {findings.map((f) => (
              <li key={f.label} className={f.severity === 'warn' ? 'g21-warn' : undefined}>
                {f.label}: {f.count}
              </li>
            ))}
          </ul>
        )}
        {removed ? <p>Temizleyicinin kaldırdığı öğe/öznitelik: {removed}</p> : null}
      </div>
    </details>
  )
}

interface DocBarProps {
  /** Konum etiketi (ör. "Sayfa 3 / 12") ve 0-1 ilerleme */
  position?: { label: string; ratio: number }
  /** Dil algılama için metin örneği */
  sampleText?: string
  findings?: readonly ScanFinding[]
  removed?: number
  /** Alıntı yalnızca bu kabın içindeki seçimden alınır */
  quoteFrom?: RefObject<HTMLElement | null>
  /** Alıntı kaynak etiketi; fonksiyon ise seçim anında hesaplanır (ör. sayfa numarası) */
  source?: string | (() => string)
  children?: ReactNode
}

export function DocBar({ position, sampleText, findings, removed, quoteFrom, source, children }: DocBarProps) {
  const language = useMemo(() => (sampleText ? detectLanguage(sampleText) : null), [sampleText])
  const [note, setNote] = useState<string | null>(null)

  const quote = () => {
    const container = quoteFrom?.current
    const selection = window.getSelection()
    const text = selection && !selection.isCollapsed ? selection.toString().trim() : ''
    const range = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null
    if (!container || !text || !range || !container.contains(range.commonAncestorContainer)) {
      setNote('Önce belgede alıntılanacak metni seç.')
      return
    }
    quoteToDraft({ text, source: typeof source === 'function' ? source() : (source ?? '') })
    setNote('Alıntı mesaj kutusuna eklendi.')
  }

  useEffect(() => {
    if (!note) return
    const t = setTimeout(() => setNote(null), 2500)
    return () => clearTimeout(t)
  }, [note])

  return (
    <div className="g21-bar">
      {position && (
        <div className="g21-pos">
          <span>{position.label}</span>
          <progress max={1} value={position.ratio} aria-label={`Belgedeki konum: ${position.label}`} />
        </div>
      )}
      {children}
      <span className="g21-bar-spacer" />
      {language && (
        <span className="g21-chip" title={`Algılanan dil (güven %${Math.round(language.confidence * 100)})`}>
          <Languages size={13} aria-hidden="true" /> {language.name}
        </span>
      )}
      {findings && <ScanBadge findings={findings} removed={removed} />}
      {quoteFrom && (
        <button type="button" className="btn small" onMouseDown={(e) => e.preventDefault()} onClick={quote} title="Seçili metni sohbete alıntı olarak ekle">
          <Quote size={14} aria-hidden="true" /> Alıntıla
        </button>
      )}
      {note && (
        <p className="g21-note" role="status">
          {note}
        </p>
      )}
    </div>
  )
}

/** Temizlenmiş belge parçasını gösterir (yalnızca sanitizeDocHtml çıktısı verilir). */
export function CleanHtml({ fragment, className }: { fragment: DocumentFragment; className?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    ref.current?.replaceChildren(fragment.cloneNode(true))
  }, [fragment])
  return <div ref={ref} className={`g21-doc prose${className ? ` ${className}` : ''}`} />
}

export function Loading({ label = 'Yükleniyor...' }: { label?: string }) {
  return (
    <p className="muted pad" role="status">
      {label}
    </p>
  )
}

export function Failure({ error }: { error: unknown }) {
  return (
    <p className="error-text pad" role="alert">
      {errorMessage(error)}
    </p>
  )
}
