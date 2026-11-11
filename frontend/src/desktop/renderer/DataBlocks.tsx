// [g8] Veri/kod blokları: kod oyun alanı (#110, SandboxedPreview + izinli betik), JSON görüntüleyici katlanabilir
//      ağaç + yol kopyalama (#111), JSON'dan şema çıkarma (#112), diff artifact (#113, mevcut DiffView yeniden
//      kullanılır). GÜVENLİK: oyun alanı yalnızca ağsız/opak-origin iframe'de ve yalnızca kullanıcı betiğe izin
//      verince çalışır; JSON/diff React ile metin olarak çizilir.
import { Braces, Code2, FileDiff } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { FileChange } from './chat-state.ts'
import { CopyButton } from './CopyButton.tsx'
import { DiffView, DIFF_MODES, type DiffMode } from './DiffView.tsx'
import { usePersistentChoice } from './persisted.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { inferSchema } from './rich-data.ts'
import { BlockFrame, ExportHtmlButton, parseJsonBlock, RawFallback } from './rich-shared.tsx'
import { escapeHtml } from './rich-data.ts'
import { SandboxedPreview } from './SandboxedPreview.tsx'

// --- Kod oyun alanı (#110) ----------------------------------------------------

interface PlaygroundSpec {
  html?: string
  css?: string
  js?: string
}

/** Oyun alanı parçalarını izole iframe gövdesine birleştirir (betik yalnızca kullanıcı izniyle çalışır). */
function composeDoc({ html = '', css = '', js = '' }: PlaygroundSpec): string {
  return `${css ? `<style>${css}</style>` : ''}${html}${js ? `<script>${js}<\/script>` : ''}`
}

function PlaygroundView({ initial }: { initial: PlaygroundSpec }) {
  const [html, setHtml] = useState(initial.html ?? '')
  const [css, setCss] = useState(initial.css ?? '')
  const [js, setJs] = useState(initial.js ?? '')
  const [tab, setTab] = useState<'html' | 'css' | 'js'>('html')
  const doc = useMemo(() => composeDoc({ html, css, js }), [html, css, js])
  const value = tab === 'html' ? html : tab === 'css' ? css : js
  const setValue = tab === 'html' ? setHtml : tab === 'css' ? setCss : setJs

  return (
    <BlockFrame
      icon={<Code2 size={14} aria-hidden="true" />}
      label="Kod oyun alanı"
      className="g8-playground"
      tools={<ExportHtmlButton title="Oyun alanı" getBody={() => doc} />}
    >
      <div className="g8-tablist" role="tablist">
        {(['html', 'css', 'js'] as const).map((t) => (
          <button key={t} type="button" role="tab" aria-selected={t === tab} className={`g8-tab${t === tab ? ' active' : ''}`} onClick={() => setTab(t)}>
            {t.toUpperCase()}
          </button>
        ))}
      </div>
      <textarea
        className="g8-code-input"
        value={value}
        spellCheck={false}
        onChange={(e) => setValue(e.target.value)}
        aria-label={`${tab.toUpperCase()} kaynağı`}
        rows={6}
      />
      {/* Betik varsayılan kapalı; kullanıcı "Betikleri çalıştır" deyince yalnızca bu önizlemede çalışır (ağ yok). */}
      <SandboxedPreview content={doc} kind="html" title="Oyun alanı önizlemesi" allowScriptsToggle height={260} />
    </BlockFrame>
  )
}

function PlaygroundBlock({ code }: RichBlockProps) {
  // JSON { html, css, js } ise parçalara ayır; değilse tüm metin HTML kabul edilir.
  const parsed = parseJsonBlock<PlaygroundSpec>(code)
  const initial = parsed.ok && parsed.data && typeof parsed.data === 'object' && !Array.isArray(parsed.data) ? parsed.data : { html: code }
  return <PlaygroundView initial={initial} />
}

// --- JSON görüntüleyici + şema (#111, #112) -----------------------------------

const MAX_JSON_DEPTH = 12

function preview(value: unknown): string {
  if (Array.isArray(value)) return `[${value.length}]`
  if (value && typeof value === 'object') return `{${Object.keys(value).length}}`
  return typeof value === 'string' ? `"${value}"` : String(value)
}

function JsonNode({ label, value, path, depth }: { label: string; value: unknown; path: string; depth: number }) {
  const isBranch = value !== null && typeof value === 'object'
  if (isBranch && depth < MAX_JSON_DEPTH) {
    const entries = Array.isArray(value)
      ? value.map((v, i) => ({ k: String(i), v, p: `${path}[${i}]` }))
      : Object.entries(value as Record<string, unknown>).map(([k, v]) => ({ k, v, p: /^[A-Za-z_$][\w$]*$/.test(k) ? `${path}.${k}` : `${path}[${JSON.stringify(k)}]` }))
    return (
      <details className="g8-json-branch" open={depth < 1}>
        <summary>
          <span className="g8-json-key">{label}</span>
          <span className="g8-json-preview">{preview(value)}</span>
          <CopyButton className="g8-json-copy" label="Yolu kopyala" getText={() => path} />
        </summary>
        <div className="g8-json-children">
          {entries.map((e) => (
            <JsonNode key={e.p} label={e.k} value={e.v} path={e.p} depth={depth + 1} />
          ))}
        </div>
      </details>
    )
  }
  return (
    <div className="g8-json-leaf">
      <span className="g8-json-key">{label}:</span>
      <span className={`g8-json-val g8-json-${value === null ? 'null' : typeof value}`}>{preview(value)}</span>
      <CopyButton className="g8-json-copy" label="Yolu kopyala" getText={() => path} />
    </div>
  )
}

function JsonView({ data }: { data: unknown }) {
  const [mode, setMode] = useState<'tree' | 'schema'>('tree')
  const schema = useMemo(() => (mode === 'schema' ? inferSchema(data) : ''), [mode, data])
  return (
    <BlockFrame
      icon={<Braces size={14} aria-hidden="true" />}
      label="JSON"
      className="g8-json"
      tools={
        <>
          <button type="button" className={`action-btn${mode === 'tree' ? ' active' : ''}`} onClick={() => setMode('tree')}>
            Ağaç
          </button>
          <button type="button" className={`action-btn${mode === 'schema' ? ' active' : ''}`} onClick={() => setMode('schema')}>
            Şema
          </button>
          {mode === 'schema' && <CopyButton className="action-btn" label="Şemayı kopyala" getText={() => schema} />}
        </>
      }
    >
      {mode === 'tree' ? (
        <div className="g8-json-tree">
          <JsonNode label="$" value={data} path="$" depth={0} />
        </div>
      ) : (
        <pre className="g8-schema">{schema}</pre>
      )}
    </BlockFrame>
  )
}

function JsonBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<unknown>(code)
  if (!parsed.ok) return <RawFallback code={code} note={parsed.error} />
  return <JsonView data={parsed.data} />
}

// --- Diff artifact (#113) -----------------------------------------------------

interface DiffSpec {
  path?: string
  before: string
  after: string
}

function DiffViewBlock({ spec }: { spec: DiffSpec }) {
  const [mode, setMode] = usePersistentChoice<DiffMode>('g8-diff-mode', DIFF_MODES, 'unified')
  const change: FileChange = { path: spec.path ?? 'metin', before: spec.before ?? '', after: spec.after ?? '' }
  return (
    <BlockFrame
      icon={<FileDiff size={14} aria-hidden="true" />}
      label={change.path}
      className="g8-diff"
      tools={
        <>
          <button type="button" className={`action-btn${mode === 'unified' ? ' active' : ''}`} onClick={() => setMode('unified')}>
            Birleşik
          </button>
          <button type="button" className={`action-btn${mode === 'split' ? ' active' : ''}`} onClick={() => setMode('split')}>
            Yan yana
          </button>
        </>
      }
    >
      <DiffView change={change} mode={mode} />
    </BlockFrame>
  )
}

function DiffBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<DiffSpec>(code)
  if (!parsed.ok || typeof parsed.data?.after !== 'string' || typeof parsed.data?.before !== 'string')
    return <RawFallback code={code} note={parsed.ok ? 'before ve after metinleri gerekli' : parsed.error} />
  return <DiffViewBlock spec={parsed.data} />
}

registerRichBlock('playground', PlaygroundBlock)
registerRichBlock('json', JsonBlock)
registerRichBlock('diff', DiffBlock)
