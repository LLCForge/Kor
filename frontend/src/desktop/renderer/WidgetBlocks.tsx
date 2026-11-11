// [g8] Küçük etkileşimli widget'lar: renk paleti (#114, hex kopyalama), ikon/emoji ızgarası (#115), quiz
//      (#116, yerel skor), progress/adım göstergesi (#117), gömülü sözlük/terim balonu (#118). İçerik React ile
//      metin olarak çizilir; renk değerleri cssColor ile süzülür (stil enjeksiyonu önlenir); ağ yok.
import { Check, HelpCircle, Hash, Palette, Smile } from 'lucide-react'
import { useMemo, useState } from 'react'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { cssColor } from './rich-sanitize.ts'
import { BlockFrame, parseJsonBlock, RawFallback } from './rich-shared.tsx'
import { percentText } from './ui-locale.ts'

/** Tıklanınca metni panoya kopyalayan ve kısa süre onay gösteren düğme. */
function CopyChip({ text, children, className, title }: { text: string; children: React.ReactNode; className: string; title: string }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 1200)
    } catch {
      setCopied(false)
    }
  }
  return (
    <button type="button" className={className} onClick={() => void copy()} title={title} aria-label={`${title}: ${text}`}>
      {copied ? <Check size={14} aria-hidden="true" /> : children}
    </button>
  )
}

// --- Renk paleti (#114) -------------------------------------------------------

interface PaletteSpec {
  colors: (string | { hex: string; name?: string })[]
}

function PaletteBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<PaletteSpec | string[]>(code)
  if (!parsed.ok) return <RawFallback code={code} note={parsed.error} />
  const list = Array.isArray(parsed.data) ? parsed.data : parsed.data?.colors
  if (!Array.isArray(list)) return <RawFallback code={code} note="colors dizisi gerekli" />
  const colors = list.map((c) => (typeof c === 'string' ? { hex: c } : c)).filter((c) => c && typeof c.hex === 'string')
  return (
    <BlockFrame icon={<Palette size={14} aria-hidden="true" />} label="Renk paleti" className="g8-palette">
      <div className="g8-grid g8-swatches">
        {colors.map((c, i) => {
          const safe = cssColor(c.hex) // yalnızca güvenli renk değeri stile girer
          return (
            <CopyChip key={i} text={c.hex} className="g8-swatch" title="Hex'i kopyala">
              <span className="g8-swatch-chip" style={{ background: safe ?? 'transparent' }} aria-hidden="true" />
              <span className="g8-swatch-hex">{c.hex}</span>
              {c.name && <span className="g8-swatch-name">{c.name}</span>}
            </CopyChip>
          )
        })}
      </div>
    </BlockFrame>
  )
}

// --- İkon / emoji ızgarası (#115) ---------------------------------------------

interface IconsSpec {
  icons: (string | { char: string; name?: string })[]
}

function IconsBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<IconsSpec | string[]>(code)
  if (!parsed.ok) return <RawFallback code={code} note={parsed.error} />
  const list = Array.isArray(parsed.data) ? parsed.data : parsed.data?.icons
  if (!Array.isArray(list)) return <RawFallback code={code} note="icons dizisi gerekli" />
  const icons = list.map((ic) => (typeof ic === 'string' ? { char: ic } : ic)).filter((ic) => ic && typeof ic.char === 'string')
  return (
    <BlockFrame icon={<Smile size={14} aria-hidden="true" />} label="İkonlar" className="g8-icons">
      <div className="g8-grid g8-icon-grid">
        {icons.map((ic, i) => (
          <CopyChip key={i} text={ic.char} className="g8-icon-cell" title={ic.name ? `${ic.name} kopyala` : 'Kopyala'}>
            <span className="g8-icon-char" aria-hidden="true">
              {ic.char}
            </span>
            {ic.name && <span className="g8-icon-name">{ic.name}</span>}
          </CopyChip>
        ))}
      </div>
    </BlockFrame>
  )
}

// --- Quiz (#116) --------------------------------------------------------------

interface QuizSpec {
  title?: string
  questions: { q: string; options: string[]; answer: number }[]
}

function QuizView({ spec }: { spec: QuizSpec }) {
  const questions = spec.questions.filter((q) => q && Array.isArray(q.options))
  const [picks, setPicks] = useState<Record<number, number>>({})
  const [checked, setChecked] = useState(false)
  const score = useMemo(() => questions.reduce((n, q, i) => (picks[i] === q.answer ? n + 1 : n), 0), [questions, picks, checked])
  return (
    <BlockFrame icon={<HelpCircle size={14} aria-hidden="true" />} label={spec.title ?? 'Quiz'} className="g8-quiz">
      <ol className="g8-quiz-list">
        {questions.map((q, i) => (
          <li key={i} className="g8-quiz-q">
            <p className="g8-quiz-text">{q.q}</p>
            <div className="g8-quiz-opts">
              {q.options.map((opt, oi) => {
                const state = checked ? (oi === q.answer ? ' correct' : picks[i] === oi ? ' wrong' : '') : ''
                return (
                  <label key={oi} className={`g8-quiz-opt${picks[i] === oi ? ' picked' : ''}${state}`}>
                    <input
                      type="radio"
                      name={`q${i}`}
                      checked={picks[i] === oi}
                      disabled={checked}
                      onChange={() => setPicks((prev) => ({ ...prev, [i]: oi }))}
                    />
                    {opt}
                  </label>
                )
              })}
            </div>
          </li>
        ))}
      </ol>
      <div className="g8-quiz-foot">
        {checked ? (
          <>
            <strong role="status">
              Skor: {score} / {questions.length}
            </strong>
            <button type="button" className="action-btn" onClick={() => { setChecked(false); setPicks({}) }}>
              Yeniden
            </button>
          </>
        ) : (
          <button type="button" className="action-btn" onClick={() => setChecked(true)} disabled={Object.keys(picks).length === 0}>
            Kontrol et
          </button>
        )}
      </div>
    </BlockFrame>
  )
}

function QuizBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<QuizSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.questions)) return <RawFallback code={code} note={parsed.ok ? 'questions dizisi gerekli' : parsed.error} />
  return <QuizView spec={parsed.data} />
}

// --- Progress / adım göstergesi (#117) ----------------------------------------

interface ProgressSpec {
  label?: string
  value?: number
  max?: number
  ring?: boolean
  steps?: string[]
  current?: number
}

function ProgressBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<ProgressSpec>(code)
  if (!parsed.ok || typeof parsed.data !== 'object' || parsed.data === null) return <RawFallback code={code} note={parsed.ok ? 'Geçersiz içerik' : parsed.error} />
  const spec = parsed.data
  if (Array.isArray(spec.steps)) {
    const current = Math.max(0, Math.min(spec.steps.length, spec.current ?? 0))
    return (
      <BlockFrame icon={<Hash size={14} aria-hidden="true" />} label={spec.label ?? 'İlerleme'} className="g8-progress">
        <ol className="g8-steps">
          {spec.steps.map((s, i) => (
            <li key={i} className={`g8-step${i < current ? ' done' : i === current ? ' active' : ''}`}>
              <span className="g8-step-num" aria-hidden="true">
                {i + 1}
              </span>
              <span>{s}</span>
            </li>
          ))}
        </ol>
      </BlockFrame>
    )
  }
  const max = spec.max && spec.max > 0 ? spec.max : 100
  const pct = Math.max(0, Math.min(100, Math.round(((spec.value ?? 0) / max) * 100)))
  return (
    <BlockFrame icon={<Hash size={14} aria-hidden="true" />} label={spec.label ?? 'İlerleme'} className="g8-progress">
      {spec.ring ? (
        <Ring pct={pct} />
      ) : (
        <div className="g8-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
          <span className="g8-bar-fill" style={{ width: `${pct}%` }} />
          <span className="g8-bar-label">{percentText(pct)}</span>
        </div>
      )}
    </BlockFrame>
  )
}

/** Yüzdeyi halka olarak çizen, bizim ürettiğimiz (güvenli) SVG. */
function Ring({ pct }: { pct: number }) {
  const r = 34
  const circ = 2 * Math.PI * r
  return (
    <svg className="g8-ring" viewBox="0 0 80 80" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
      <circle cx="40" cy="40" r={r} className="g8-ring-track" />
      <circle cx="40" cy="40" r={r} className="g8-ring-fill" strokeDasharray={circ} strokeDashoffset={circ * (1 - pct / 100)} transform="rotate(-90 40 40)" />
      <text x="40" y="45" textAnchor="middle" className="g8-ring-text">
        %{pct}
      </text>
    </svg>
  )
}

// --- Sözlük / terim balonu (#118) ---------------------------------------------

interface GlossarySpec {
  text?: string
  terms: { term: string; definition: string }[]
}

function GlossaryBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<GlossarySpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.terms)) return <RawFallback code={code} note={parsed.ok ? 'terms dizisi gerekli' : parsed.error} />
  const terms = parsed.data.terms.filter((t) => t && typeof t.term === 'string')
  return (
    <BlockFrame icon={<HelpCircle size={14} aria-hidden="true" />} label="Sözlük" className="g8-glossary">
      {parsed.data.text && <p className="g8-glossary-text">{parsed.data.text}</p>}
      <div className="g8-term-list">
        {terms.map((t, i) => (
          // Terimin üstüne gelince/odaklanınca tanım balonu CSS ile gösterilir (erişilebilir: odaklanılabilir).
          <span key={i} className="g8-term" tabIndex={0}>
            {t.term}
            <span className="g8-term-pop" role="tooltip">
              {t.definition}
            </span>
          </span>
        ))}
      </div>
    </BlockFrame>
  )
}

registerRichBlock('palette', PaletteBlock)
registerRichBlock('colors', PaletteBlock)
registerRichBlock('icons', IconsBlock)
registerRichBlock('quiz', QuizBlock)
registerRichBlock('progress', ProgressBlock)
registerRichBlock('glossary', GlossaryBlock)
