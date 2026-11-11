// [g21] Zengin metin farkı (#179): iki belgenin (metin, Markdown, PDF, DOCX, EPUB) çıkarılmış metnini kelime
//      bazında karşılaştırır. Değişmeyen uzun bölümler kısaltılır; fark React ile metin olarak çizilir.
import { GitCompare, X } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { api } from '../api.ts'
import { loadRichBundle } from '../rich-lazy.ts'
import { convertDocx, loadEpubChapter } from './DocViewers.tsx'
import { Failure, Loading, useDocQuery } from './DocChrome.tsx'
import { openEpub } from './epub.ts'
import { g21KindOf } from './kinds.ts'
import type { G21PdfApi } from './pdf-entry.ts'
import { MAX_DIFF_CHARS, wordDiff } from './word-diff.ts'
import { uiLocale } from '../ui-locale.ts'

const MAX_PDF_PAGES = 500
const KEEP_CONTEXT = 200

async function pdfText(path: string): Promise<string> {
  const lib = await loadRichBundle<G21PdfApi>('./g21-pdf.js', '__g21Pdf')
  const bytes = await api.g21ReadDoc(path)
  const worker = lib.createWorker()
  const task = lib.getDocument({ data: bytes, worker: worker.pdfWorker, enableXfa: false, verbosity: 0 })
  try {
    const doc = await task.promise
    let text = ''
    for (let n = 1; n <= Math.min(doc.numPages, MAX_PDF_PAGES) && text.length < MAX_DIFF_CHARS; n++) {
      const content = await (await doc.getPage(n)).getTextContent()
      text += content.items.map((i) => ('str' in i ? i.str + (i.hasEOL ? '\n' : ' ') : '')).join('') + '\n\n'
    }
    return text
  } finally {
    await task.destroy()
    worker.terminate()
  }
}

/** Dosyanın karşılaştırılabilir düz metni (türüne göre çıkarılır). */
export async function documentText(path: string): Promise<string> {
  const kind = g21KindOf(path)
  if (kind === 'pdf') return pdfText(path)
  if (kind === 'docx') return (await convertDocx(path)).text
  if (kind === 'epub') {
    const book = await openEpub(await api.g21ReadDoc(path))
    let text = ''
    for (let i = 0; i < book.chapters.length && text.length < MAX_DIFF_CHARS; i++) text += `${(await loadEpubChapter(book, i)).text}\n\n`
    return text
  }
  const preview = await api.g4.preview(path)
  if (preview.kind === 'text') return preview.content
  throw new Error(`${path}: bu dosya türü metin olarak karşılaştırılamaz`)
}

function Same({ value }: { value: string }) {
  if (value.length <= KEEP_CONTEXT * 3) return <span>{value}</span>
  return (
    <span>
      {value.slice(0, KEEP_CONTEXT)}
      <span className="g21-skip">[… {(value.length - KEEP_CONTEXT * 2).toLocaleString(uiLocale())} karakter aynı …]</span>
      {value.slice(-KEEP_CONTEXT)}
    </span>
  )
}

export function ComparePanel({ path, onClose }: { path: string; onClose: () => void }) {
  const [other, setOther] = useState('')
  const [target, setTarget] = useState<string | null>(null)
  const files = useSWR('g21:compare-files', () => api.g4.listAllFiles(true), { revalidateOnFocus: false })
  const result = useDocQuery(target ? ['g21', 'compare', path, target] : null, async () => {
    const [before, after] = await Promise.all([documentText(path), documentText(target ?? '')])
    return wordDiff(before, after)
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const value = other.trim()
    if (value) setTarget(value)
  }

  return (
    <section className="g21-compare" aria-label="Belge karşılaştırma">
      <form className="g21-compare-form" onSubmit={submit}>
        <GitCompare size={14} aria-hidden="true" />
        <input
          className="g8-input"
          list="g21-compare-files"
          value={other}
          onChange={(e) => setOther(e.target.value)}
          placeholder="Karşılaştırılacak dosya (ör. docs/v2.docx)"
          aria-label="Karşılaştırılacak dosyanın yolu"
        />
        <datalist id="g21-compare-files">
          {files.data?.files.slice(0, 5000).map((f) => (
            <option key={f} value={f} />
          ))}
        </datalist>
        <button type="submit" className="btn small" disabled={!other.trim()}>
          Karşılaştır
        </button>
        <button type="button" className="action-btn" onClick={onClose} aria-label="Karşılaştırmayı kapat" title="Kapat">
          <X size={15} aria-hidden="true" />
        </button>
      </form>
      {target && result.error && <Failure error={result.error} />}
      {target && !result.error && result.data === undefined && <Loading label="Belgeler karşılaştırılıyor..." />}
      {result.data === null && <p className="g21-muted g21-pad">Belgeler çok farklı ya da çok uzun; kelime farkı sınırı (süre/düzenleme sayısı) aşıldı.</p>}
      {result.data && (
        <>
          <p className="g21-muted g21-pad" role="status">
            <span className="stat-add">+{result.data.addedWords}</span> <span className="stat-del">-{result.data.removedWords}</span> kelime
            {result.data.truncated ? ` (ilk ${MAX_DIFF_CHARS.toLocaleString(uiLocale())} karakter karşılaştırıldı)` : ''} · {path} → {target}
          </p>
          <div className="g21-diff" tabIndex={0} aria-label="Kelime farkı">
            {result.data.parts.map((p, i) =>
              p.kind === 'added' ? <ins key={i}>{p.value}</ins> : p.kind === 'removed' ? <del key={i}>{p.value}</del> : <Same key={i} value={p.value} />,
            )}
          </div>
        </>
      )}
    </section>
  )
}
