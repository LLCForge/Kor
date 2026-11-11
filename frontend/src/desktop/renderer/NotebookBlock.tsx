// [g20] ```notebook / ```ipynb bloğu: .ipynb hücrelerini çıktılarıyla sohbette gösterir (#121). Gövde ya satır içi
//      notebook JSON'u ya da çalışma dizinindeki .ipynb yoludur (ana süreç yol hapsiyle okur). Markdown hücreleri
//      biçimli (#139); görsel/HTML/SVG çıktılar SandboxedPreview'de (#122); Python hücreleri onaylı yeniden
//      çalıştırılır (#138; çekirdek durumu yoktur, hücre tek başına çalışır); çıktılar tek tıkla gizlenir (#140).
import { Eraser, LoaderCircle, NotebookText, RotateCcw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from './api.ts'
import { AnsiText, RunOutput, RunView } from './G20Output.tsx'
import { normalizeNotebook, notebookSource, type NbCell, type NbOutput, type NbView } from './g20-notebook.ts'
import { fromPython, recordRun, runHistory } from './g20-runs.ts'
import { Markdown } from './Markdown.tsx'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { withinRichSizeLimit } from './rich-sanitize.ts'
import { BlockFrame, RawFallback } from './rich-shared.tsx'
import { SandboxedPreview } from './SandboxedPreview.tsx'

const MAX_IMAGE_FRAME = 640

/** Görselin doğal yüksekliği (iframe betiksiz olduğundan yüksekliği kendisi bildiremez). */
function useImageHeight(src: string): number | undefined {
  const [height, setHeight] = useState<number>()
  useEffect(() => {
    const img = new Image()
    img.onload = () => setHeight(img.naturalHeight)
    img.onerror = () => setHeight(undefined)
    img.src = src
    return () => {
      img.onload = null
      img.onerror = null
    }
  }, [src])
  return height
}

function ImageOutput({ mime, base64 }: { mime: string; base64: string }) {
  const src = `data:${mime};base64,${base64}`
  const natural = useImageHeight(src)
  return (
    <SandboxedPreview
      kind="html"
      title="Hücre görsel çıktısı"
      content={`<img src="${src}" alt="Hücre çıktısı">`}
      height={natural ? Math.min(MAX_IMAGE_FRAME, natural + 20) : 240}
    />
  )
}

function OutputView({ output }: { output: NbOutput }) {
  switch (output.kind) {
    case 'stream':
      return <RunOutput chunks={[{ stream: output.stream, text: output.text }]} />
    case 'error':
      return (
        <pre className="g20-output">
          <span className="g20-err">
            <AnsiText text={output.text} />
          </span>
        </pre>
      )
    case 'html':
      return <SandboxedPreview kind="html" title="Hücre HTML çıktısı" content={output.html} allowScriptsToggle />
    case 'svg':
      return <SandboxedPreview kind="svg" title="Hücre SVG çıktısı" content={output.svg} />
    case 'image':
      return <ImageOutput mime={output.mime} base64={output.base64} />
    case 'markdown':
      return (
        <div className="g20-nb-md">
          <Markdown text={output.text} />
        </div>
      )
    case 'text':
      return (
        <pre className="g20-output">
          <AnsiText text={output.text} />
        </pre>
      )
  }
}

function CodeCell({ cell, index, language, showOutputs, historyKey }: { cell: NbCell; index: number; language: string; showOutputs: boolean; historyKey: string }) {
  const key = `${historyKey}#${index}`
  const [runs, setRuns] = useState(() => runHistory(key))
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null)
  const python = language === 'python'

  const rerun = async () => {
    setBusy(true)
    setMessage(null)
    try {
      const result = await api.g20RunPython(cell.source)
      if (!result) setMessage({ text: 'Çalıştırma iptal edildi.', error: false })
      else setRuns(recordRun(key, fromPython(result)))
    } catch (err) {
      setMessage({ text: errorMessage(err), error: true })
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="g20-nb-cell" aria-label={`Hücre ${index + 1} (kod)`}>
      <div className="g20-nb-cell-head">
        <span className="g20-nb-count">[{cell.count ?? ' '}]</span>
        <button
          type="button"
          className="action-btn"
          onClick={() => void rerun()}
          disabled={busy || !python || !cell.source.trim()}
          aria-label="Hücreyi yeniden çalıştır (onay istenir)"
          title={python ? 'Hücreyi yeniden çalıştır: yerel Python, onay istenir; önceki hücrelerin durumu yoktur' : 'Yalnızca Python not defterlerinde'}
        >
          {busy ? <LoaderCircle size={14} className="spin" aria-hidden="true" /> : <RotateCcw size={14} aria-hidden="true" />}
        </button>
      </div>
      <pre className="g20-nb-src">{cell.source}</pre>
      {showOutputs && cell.outputs.map((o, i) => <OutputView key={i} output={o} />)}
      {message && (
        <p className={message.error ? 'g20-note error-text' : 'g20-note'} role={message.error ? 'alert' : 'status'}>
          {message.text}
        </p>
      )}
      {runs[0] && (
        <div className="g20-nb-rerun">
          <p className="g20-note">Yeniden çalıştırma sonucu</p>
          <RunView record={runs[0]} />
        </div>
      )}
    </section>
  )
}

function NotebookView({ view, label, historyKey }: { view: NbView; label: string; historyKey: string }) {
  const [showOutputs, setShowOutputs] = useState(true)
  const tools = (
    <button
      type="button"
      className="action-btn"
      onClick={() => setShowOutputs((v) => !v)}
      aria-pressed={!showOutputs}
      aria-label={showOutputs ? 'Çıktıları gizle' : 'Çıktıları göster'}
      title={showOutputs ? 'Çıktıları gizle' : 'Çıktıları göster'}
    >
      <Eraser size={15} aria-hidden="true" />
    </button>
  )
  return (
    <BlockFrame icon={<NotebookText size={15} aria-hidden="true" />} label={`${label} · ${view.cells.length} hücre · ${view.language}`} tools={tools} className="g20-notebook">
      {view.cells.length === 0 && <p className="g20-note">Not defteri boş.</p>}
      {view.cells.map((cell, i) =>
        cell.type === 'markdown' ? (
          <section key={i} className="g20-nb-cell g20-nb-md" aria-label={`Hücre ${i + 1} (metin)`}>
            <Markdown text={cell.source} />
          </section>
        ) : cell.type === 'raw' ? (
          <pre key={i} className="g20-nb-src">
            {cell.source}
          </pre>
        ) : (
          <CodeCell key={i} cell={cell} index={i} language={view.language} showOutputs={showOutputs} historyKey={historyKey} />
        ),
      )}
    </BlockFrame>
  )
}

function NotebookBlock({ code }: RichBlockProps) {
  const source = useMemo(() => notebookSource(code), [code])
  const path = source.kind === 'path' ? source.path : null
  const { data, error, isLoading } = useSWR(path ? ['g20:notebook', path] : null, ([, p]: [string, string]) => api.g20ReadNotebook(p), {
    revalidateOnFocus: false,
  })
  const text = source.kind === 'json' ? source.text : data
  const parsed = useMemo((): { view: NbView } | { error: string } | null => {
    if (text === undefined) return null
    if (source.kind === 'json' && !withinRichSizeLimit(text)) return { error: 'Satır içi not defteri çok büyük; dosya yolu verin.' }
    try {
      return { view: normalizeNotebook(JSON.parse(text)) }
    } catch (err) {
      return { error: err instanceof SyntaxError ? 'Not defteri geçerli JSON değil' : errorMessage(err) }
    }
  }, [text, source.kind])

  if (source.kind === 'pending') return <RawFallback code={code} note="Not defteri yolu bekleniyor (.ipynb)" />
  if (error) return <RawFallback code={code} note={`Not defteri okunamadı: ${errorMessage(error)}`} />
  if (isLoading || !parsed) return <RawFallback code={code} note="Not defteri yükleniyor…" />
  if ('error' in parsed) return <RawFallback code={code} note={parsed.error} />
  return <NotebookView view={parsed.view} label={path ?? 'Not defteri'} historyKey={path ?? code.slice(0, 200)} />
}

registerRichBlock('notebook', NotebookBlock)
registerRichBlock('ipynb', NotebookBlock)
