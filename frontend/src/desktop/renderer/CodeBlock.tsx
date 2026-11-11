// [g3] Kod bloğu: satır numarası, satır kaydırma, Markdown olarak kopyalama, dosyaya kaydetme ve kabuk komutunu çalıştırma.
import type { Element, ElementContent } from 'hast'
import { FileCode, GitPullRequestArrow, ListOrdered, LoaderCircle, Play, Save, WrapText } from 'lucide-react'
import { useMemo, useState, type ReactNode } from 'react'
import { isShellLanguage } from '../../core/code-langs.ts'
import type { SnippetResult } from '../../core/snippet-runner.ts'
import { api, errorMessage } from './api.ts'
import { CopyButton } from './CopyButton.tsx'
import { G17CodeDownload } from './G17Blocks.tsx' // [g17]
import { g3Actions } from './g3-bridge.ts'
import { LongOutput } from './LongOutput.tsx'
import { codeFence, hastText, splitHastLines } from './markdown-utils.ts'
import { parseFence } from './fence-info.ts'
import { insertIntoDraft } from './g21/quote.ts'

/** Vurgulanmış kodda yalnızca sınıflı span'ler bulunur; başka öznitelik aktarılmaz. */
function renderHast(node: ElementContent, key: number): ReactNode {
  if (node.type === 'text') return node.value
  if (node.type !== 'element') return null
  const cls = node.properties.className
  return (
    <span key={key} className={Array.isArray(cls) ? cls.join(' ') : undefined}>
      {node.children.map(renderHast)}
    </span>
  )
}

interface ToolButtonProps {
  label: string
  pressed?: boolean
  disabled?: boolean
  onClick: () => void
  children: ReactNode
}

function ToolButton({ label, pressed, disabled, onClick, children }: ToolButtonProps) {
  return (
    <button
      type="button"
      className={`code-copy${pressed ? ' g3-on' : ''}`}
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      {...(pressed === undefined ? {} : { 'aria-pressed': pressed })}
    >
      {children}
    </button>
  )
}

type Status = { kind: 'idle' } | { kind: 'busy' } | { kind: 'note'; text: string } | { kind: 'error'; text: string }

export function CodeBlock({ code, language: rawLanguage, meta = null }: { code: Element; language: string; meta?: string | null }) {
  // §23.4: başlıkta dil + dosya yolu (```ts:src/a.ts, title=…, path=…).
  const { language, path } = parseFence(rawLanguage, meta)
  const [lineNumbers, setLineNumbers] = useState(false)
  const [wrap, setWrap] = useState(false)
  const [status, setStatus] = useState<Status>({ kind: 'idle' })
  const [run, setRun] = useState<SnippetResult | null>(null)
  const text = useMemo(() => hastText(code).replace(/\n$/, ''), [code])
  const lines = useMemo(() => {
    const all = splitHastLines(code.children)
    // Kod çoğu zaman satır sonuyla biter; son boş satır numaralandırılmaz.
    return all.length > 1 && all.at(-1)?.length === 0 ? all.slice(0, -1) : all
  }, [code])
  const shell = isShellLanguage(language)
  const busy = status.kind === 'busy'

  const save = async () => {
    setStatus({ kind: 'busy' })
    try {
      const saved = await api.g3SaveCode(text, language)
      setStatus(saved ? { kind: 'note', text: `Kaydedildi: ${saved}` } : { kind: 'idle' })
      if (saved) g3Actions.current?.refreshFiles()
    } catch (err) {
      setStatus({ kind: 'error', text: errorMessage(err) })
    }
  }

  const execute = async () => {
    setStatus({ kind: 'busy' })
    try {
      const result = await api.g3RunCode(text, language)
      if (result) setRun(result)
      setStatus({ kind: 'idle' })
    } catch (err) {
      setStatus({ kind: 'error', text: errorMessage(err) })
    }
  }

  return (
    <div className={`code-block g3-code${wrap ? ' g3-wrap' : ''}${lineNumbers ? ' g3-numbered' : ''}`}>
      <div className="code-head">
        <span className="g3-code-lang">{language || 'kod'}</span>
        {path && (
          <span className="kor-code-path" translate="no" title={path}>
            {path}
          </span>
        )}
        <div className="g3-code-tools">
          <ToolButton label="Satır numaraları" pressed={lineNumbers} onClick={() => setLineNumbers((v) => !v)}>
            <ListOrdered size={15} aria-hidden="true" />
          </ToolButton>
          <ToolButton label="Satırları kaydır" pressed={wrap} onClick={() => setWrap((v) => !v)}>
            <WrapText size={15} aria-hidden="true" />
          </ToolButton>
          <ToolButton label="Dosyaya kaydet" disabled={busy} onClick={() => void save()}>
            <Save size={15} aria-hidden="true" />
          </ToolButton>
          {/* [g17] kod bloğunu dile uygun uzantıyla dosya olarak indir */}
          <G17CodeDownload text={text} language={language} />
          {shell && (
            <ToolButton label="Çalıştır" disabled={busy} onClick={() => void execute()}>
              {busy ? <LoaderCircle size={15} className="spin" aria-hidden="true" /> : <Play size={15} aria-hidden="true" />}
            </ToolButton>
          )}
          <CopyButton className="code-copy" label="Markdown olarak kopyala" getText={() => codeFence(text, language)} icon={<FileCode size={15} aria-hidden="true" />} />
          <CopyButton className="code-copy" label="Kodu kopyala" getText={() => text} />
          {/* §23.4 "uygula": ajan dosyaya diff önizlemesiyle uygular (ADR 0030 dosya başına onay atlanmaz) */}
          <ToolButton
            label="Diff'e uygula"
            onClick={() => insertIntoDraft(`Bu kodu ${path ? `\`${path}\`` : 'ilgili dosyaya'} uygula (diff önizlemesiyle):\n${codeFence(text, language)}`)}
          >
            <GitPullRequestArrow size={15} aria-hidden="true" />
          </ToolButton>
        </div>
      </div>
      <pre>
        <code className={Array.isArray(code.properties.className) ? code.properties.className.join(' ') : undefined}>
          {lines.map((parts, i) => (
            <span key={i} className="g3-line">
              {parts.map(renderHast)}
              {'\n'}
            </span>
          ))}
        </code>
      </pre>
      {(status.kind === 'note' || status.kind === 'error') && (
        <p className={`g3-code-status${status.kind === 'error' ? ' error-text' : ''}`} role={status.kind === 'error' ? 'alert' : 'status'}>
          {status.text}
        </p>
      )}
      {run && (
        <div className="g3-run" aria-label="Komut çıktısı">
          <p className="g3-run-head">
            {run.timedOut ? 'Zaman aşımı: 60 sn sonra durduruldu' : `Çıkış kodu ${run.exitCode ?? '?'}`}
            {run.truncated && ' · çıktı kısaltıldı'}
          </p>
          <LongOutput text={run.output.trim() || '(çıktı yok)'} className="g3-run-output" />
        </div>
      )}
    </div>
  )
}
