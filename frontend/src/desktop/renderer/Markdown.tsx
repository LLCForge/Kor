import 'highlight.js/styles/github-dark.css'
import 'katex/dist/katex.min.css'
import type { Element, Root, RootContent } from 'hast'
import { Table2 } from 'lucide-react'
import { memo, useMemo, type ReactNode } from 'react'
import ReactMarkdown, { type Components, type Options } from 'react-markdown'
import rehypeHighlight from 'rehype-highlight'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'
import { api } from './api.ts'
import { CodeBlock } from './CodeBlock.tsx'
import { CopyButton } from './CopyButton.tsx'
import { G17TableTsv } from './G17Blocks.tsx' // [g17]
import { isDiagramLanguage, looksLikeAsciiDiagram } from './diagram-utils.ts'
import { DiagramTabs } from './DiagramTabs.tsx'
import { g3Actions } from './g3-bridge.ts'
import { hastText, normalizeMathDelimiters, parseFileRef, rehypeFileLinks, tableToMarkdown } from './markdown-utils.ts'
import { splitMarkdownBlocks } from './markdown-blocks.ts' // [g16]
import { MathView } from './MathView.tsx'
import { getRichBlock } from './rich-block-registry.ts'
import './rich-blocks.ts'

// [g3] feature 29: Bir <pre> diyagram bloğu mu (dili diyagram diliyse dili döndürür).
function preDiagramLanguage(node: RootContent): string | null {
  if (node.type !== 'element' || node.tagName !== 'pre') return null
  const code = node.children.find((c): c is Element => c.type === 'element' && c.tagName === 'code')
  const classes = code?.properties?.className
  const lang = Array.isArray(classes) ? classes.map(String).find((c) => c.startsWith('language-'))?.slice('language-'.length) : undefined
  return lang && isDiagramLanguage(lang) ? lang : null
}

const DIAGRAM_LABELS: Record<string, string> = {
  mermaid: 'Mermaid',
  dot: 'Graphviz',
  graphviz: 'Graphviz',
  plantuml: 'PlantUML',
  puml: 'PlantUML',
  nomnoml: 'nomnoml',
  wavedrom: 'WaveDrom',
}

// [g3] feature 29: Art arda gelen (≥2) diyagram bloklarını tek sekmeli kap içine alır (içerik değişmez).
function rehypeDiagramTabs() {
  return (tree: Root) => {
    const out: RootContent[] = []
    let run: Element[] = []
    let langs: string[] = []
    const flush = () => {
      if (run.length >= 2) {
        out.push({ type: 'element', tagName: 'div', properties: { dataDiagramTabs: true, dataDiagramLangs: langs.join(',') }, children: run })
      } else {
        out.push(...run)
      }
      run = []
      langs = []
    }
    for (const node of tree.children) {
      const lang = preDiagramLanguage(node)
      if (lang && node.type === 'element') {
        run.push(node)
        langs.push(lang)
        continue
      }
      if (node.type === 'text' && node.value.trim() === '' && run.length) continue // bloklar arası boşluk run'ı bölmesin
      flush()
      out.push(node)
    }
    flush()
    tree.children = out
  }
}

function FileLink({ path, line, children }: { path: string; line: number | undefined; children: ReactNode }) {
  return (
    <button
      type="button"
      className="g3-file-link"
      onClick={() => g3Actions.current?.openFile(path, line)}
      title={line ? `${path} dosyasını aç (satır ${line})` : `${path} dosyasını aç`}
    >
      {children}
    </button>
  )
}

const components: Components = {
  pre: ({ node }) => {
    const code = node?.children[0]
    if (code?.type !== 'element') return <pre>{node ? hastText(node) : null}</pre>
    const classes = code.properties.className
    const language = Array.isArray(classes)
      ? (classes.map(String).find((c) => c.startsWith('language-'))?.slice('language-'.length) ?? '')
      : ''
    // Zengin blok kayıt defterinde tek arama noktası: dil kayıtlıysa o bileşen, değilse normal kod bloğu.
    const Rich = getRichBlock(language)
    if (Rich) return <Rich code={hastText(code).replace(/\n$/, '')} language={language} />
    // remark-math blok matematiği: <pre><code class="language-math math-display">
    if (language === 'math') return <MathView tex={hastText(code)} display />
    // feature 20: etiketsiz blok kutu-çizgi ASCII şemaya benziyorsa monospace hizalı göster.
    if (!language) {
      const raw = hastText(code)
      const Ascii = looksLikeAsciiDiagram(raw) ? getRichBlock('ascii') : undefined
      if (Ascii) return <Ascii code={raw.replace(/\n$/, '')} language="ascii" />
    }
    return <CodeBlock code={code} language={language} meta={(code.data as { meta?: string } | undefined)?.meta ?? null} />
  },
  // feature 29: rehypeDiagramTabs'ın işaretlediği kap sekmeli diyagram kartına dönüşür.
  div: ({ node, children, ...props }) => {
    if (node?.properties?.dataDiagramTabs) {
      const raw = node.properties.dataDiagramLangs
      const labels = typeof raw === 'string' ? raw.split(',').map((l) => DIAGRAM_LABELS[l] ?? l) : []
      return <DiagramTabs labels={labels}>{children}</DiagramTabs>
    }
    return <div {...props}>{children}</div>
  },
  // Satır içi koddaki dosya yolu (ör. `src/a.ts:12`) tıklanınca önizlemede açılır.
  code: ({ node, children, ...props }) => {
    if (node && hastText(node) && props.className?.includes('language-math')) return <MathView tex={hastText(node)} display={false} />
    const ref = typeof children === 'string' ? parseFileRef(children) : null
    const code = <code {...props}>{children}</code>
    return ref ? (
      <FileLink path={ref.path} line={ref.line}>
        {code}
      </FileLink>
    ) : (
      code
    )
  },
  span: ({ node, children, ...props }) => {
    const path = node?.properties.dataPath
    if (typeof path !== 'string') return <span {...props}>{children}</span>
    const line = Number(node?.properties.dataLine)
    return (
      <FileLink path={path} line={Number.isInteger(line) && line > 0 ? line : undefined}>
        {children}
      </FileLink>
    )
  },
  // Bağlantılar uygulama içinde açılmaz; ana süreç yalnızca http/https adreslerini sistem tarayıcısına iletir.
  a: ({ href, children }) => (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault()
        if (href) api.openExternal(href).catch(() => undefined)
      }}
    >
      {children}
    </a>
  ),
  table: ({ node, children }) => (
    <div className="table-wrap g3-table">
      {node && (
        <div className="g3-table-tools">
          <CopyButton
            className="action-btn g3-table-copy"
            label="Tabloyu Markdown olarak kopyala"
            getText={() => tableToMarkdown(node)}
            icon={<Table2 size={14} aria-hidden="true" />}
          />
          {/* [g17] tabloyu TSV olarak kopyala */}
          <G17TableTsv node={node} />
        </div>
      )}
      <table>{children}</table>
    </div>
  ),
}

const remarkPlugins: Options['remarkPlugins'] = [remarkGfm, [remarkMath, { singleDollarTextMath: false }]]
const rehypePlugins: Options['rehypePlugins'] = [
  [rehypeHighlight, { detect: false, plainText: ['math', 'mermaid', 'dot', 'graphviz', 'plantuml', 'puml', 'nomnoml', 'wavedrom', 'ascii'] }],
  rehypeFileLinks,
  rehypeDiagramTabs,
]

// react-markdown ham HTML'i varsayılan olarak işlemez; model çıktısındaki <script> vb. metin olarak kalır (XSS koruması).
// Tek $ para birimiyle karışmasın diye kapalı; matematik $$...$$, \(...\) veya \[...\] ile yazılır.
// [g16] Artımlı ayrıştırma (241): tamamlanmış bloklar aynı dizgi kaldığı için memo'lanır; akışta yalnızca son blok yeniden çizilir.
const MarkdownBlock = memo(function MarkdownBlock({ source }: { source: string }) {
  return (
    <ReactMarkdown remarkPlugins={remarkPlugins} rehypePlugins={rehypePlugins} components={components}>
      {source}
    </ReactMarkdown>
  )
})

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  // Art arda diyagramlar aynı blokta kalır ki sekmeli karta (#29) toplanabilsin.
  const blocks = useMemo(() => splitMarkdownBlocks(normalizeMathDelimiters(text), isDiagramLanguage), [text])
  return blocks.map((block, i) => <MarkdownBlock key={i} source={block} />)
})
