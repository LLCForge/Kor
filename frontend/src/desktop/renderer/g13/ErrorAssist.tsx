// [g13] Hata mesajı açıklayıcı + yığın izi gezgini.
// Sohbette seçilen metnin yanında "Açıkla ve çöz" ve seçimdeki dosya:satır konumları çıkar; araç/komut çıktılarındaki
// dosya:satır konumları altı çizili (CSS Custom Highlight) ve tıklanabilir olur. Ortak bileşenlere (ToolCard, LongOutput)
// dokunmamak için DOM değiştirilmez: belge düzeyinde dinleyici ve caretPositionFromPoint kullanılır.
import { FileCode, WandSparkles } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { explainPrompt, findStackRefs, type StackRef } from './stack-trace.ts'

const OUTPUT_SELECTOR = 'pre.tool-output, pre.g3-run-output'
const THREAD_SELECTOR = '.g3-thread'
const HIGHLIGHT = 'g13-stack'
const MAX_TOOLBAR_REFS = 6
const TOOLBAR_WIDTH = 360

interface ErrorAssistProps {
  root: string | null
  onExplain: (prompt: string) => void
  onOpenFile: (path: string, line: number) => void
}

interface Selected {
  text: string
  top: number
  left: number
  refs: StackRef[]
}

interface Scanned {
  signature: string
  refs: Array<{ node: Text; ref: StackRef; range: Range }>
}

const baseName = (p: string) => p.slice(p.lastIndexOf('/') + 1)

export function ErrorAssist({ root, onExplain, onOpenFile }: ErrorAssistProps) {
  const [selected, setSelected] = useState<Selected | null>(null)
  // Belge dinleyicileri kök değişene kadar bir kez kurulur; güncel geri çağrı ref ile okunur.
  const openRef = useRef(onOpenFile)
  openRef.current = onOpenFile

  useEffect(() => {
    let timer = 0
    const update = () => {
      const selection = window.getSelection()
      const text = selection?.toString().trim() ?? ''
      if (!selection || selection.rangeCount === 0 || selection.isCollapsed || !text) return setSelected(null)
      const range = selection.getRangeAt(0)
      const node = range.commonAncestorContainer
      const host = node instanceof Element ? node : node.parentElement
      if (!host?.closest(THREAD_SELECTOR)) return setSelected(null)
      const rect = range.getBoundingClientRect()
      const below = rect.bottom + 48 < window.innerHeight
      setSelected({
        text,
        top: below ? rect.bottom + 6 : Math.max(8, rect.top - 44),
        left: Math.max(8, Math.min(rect.left, window.innerWidth - TOOLBAR_WIDTH)),
        refs: root ? findStackRefs(text, root).slice(0, MAX_TOOLBAR_REFS) : [],
      })
    }
    const schedule = () => {
      window.clearTimeout(timer)
      timer = window.setTimeout(update, 150)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSelected(null)
    }
    document.addEventListener('selectionchange', schedule)
    window.addEventListener('scroll', schedule, true)
    window.addEventListener('keydown', onKey)
    return () => {
      window.clearTimeout(timer)
      document.removeEventListener('selectionchange', schedule)
      window.removeEventListener('scroll', schedule, true)
      window.removeEventListener('keydown', onKey)
    }
  }, [root])

  useEffect(() => {
    if (!root) return
    const highlight = new Highlight()
    CSS.highlights.set(HIGHLIGHT, highlight)
    const scanned = new WeakMap<Element, Scanned>()

    const textNodes = (pre: Element) => {
      const nodes: Text[] = []
      const walker = document.createTreeWalker(pre, NodeFilter.SHOW_TEXT)
      for (let n = walker.nextNode(); n; n = walker.nextNode()) if (n instanceof Text) nodes.push(n)
      return nodes
    }
    // Çıktı yalnızca "daha fazla göster" ile değişir; uzunluk imzası her fare hareketinde tüm metni kurmaktan ucuzdur.
    const scan = (pre: Element): Scanned => {
      const nodes = textNodes(pre)
      const signature = `${nodes.length}:${nodes.reduce((n, t) => n + t.length, 0)}`
      const previous = scanned.get(pre)
      if (previous?.signature === signature) return previous
      for (const r of previous?.refs ?? []) highlight.delete(r.range)
      const refs = nodes.flatMap((node) =>
        findStackRefs(node.data, root).map((ref) => {
          const range = document.createRange()
          range.setStart(node, ref.start)
          range.setEnd(node, ref.end)
          highlight.add(range)
          return { node, ref, range }
        }),
      )
      const entry = { signature, refs }
      scanned.set(pre, entry)
      return entry
    }
    const refAt = (e: MouseEvent): StackRef | null => {
      const pre = e.target instanceof Element ? e.target.closest(OUTPUT_SELECTOR) : null
      if (!pre) return null
      const { refs } = scan(pre)
      if (refs.length === 0) return null
      const pos = document.caretPositionFromPoint(e.clientX, e.clientY)
      if (!pos) return null
      return refs.find((r) => r.node === pos.offsetNode && pos.offset >= r.ref.start && pos.offset < r.ref.end)?.ref ?? null
    }
    const onMove = (e: MouseEvent) => document.body.classList.toggle('g13-on-ref', refAt(e) !== null)
    const onClick = (e: MouseEvent) => {
      // Sürükleyerek seçim yapan kullanıcıyı dosyaya atlatma.
      if (e.button !== 0 || window.getSelection()?.isCollapsed === false) return
      const ref = refAt(e)
      if (!ref) return
      e.preventDefault()
      openRef.current(ref.path, ref.line)
    }
    document.addEventListener('mousemove', onMove)
    document.addEventListener('click', onClick)
    return () => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('click', onClick)
      CSS.highlights.delete(HIGHLIGHT)
      document.body.classList.remove('g13-on-ref')
    }
  }, [root])

  if (!selected) return null
  return (
    <div
      className="g13-assist"
      role="toolbar"
      aria-label="Seçili metin için eylemler"
      style={{ top: selected.top, left: selected.left }}
      // Düğmeye basmak seçimi kaldırmasın.
      onMouseDown={(e) => e.preventDefault()}
    >
      <button
        type="button"
        className="btn small primary"
        onClick={() => {
          onExplain(explainPrompt(selected.text))
          setSelected(null)
        }}
      >
        <WandSparkles size={14} aria-hidden="true" /> Açıkla ve çöz
      </button>
      {selected.refs.map((r) => (
        <button
          key={`${r.start}:${r.path}`}
          type="button"
          className="btn small g13-assist-ref"
          onClick={() => onOpenFile(r.path, r.line)}
          title={`${r.path} dosyasını aç (satır ${r.line})`}
        >
          <FileCode size={14} aria-hidden="true" />
          {baseName(r.path)}:{r.line}
        </button>
      ))}
    </div>
  )
}
