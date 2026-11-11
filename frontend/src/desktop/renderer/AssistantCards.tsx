// [g23] Mesaj, grafik, tarif, hava, harita, önizleme ve etkinlik taslağı kartları.
import { Check, ChevronLeft, ChevronRight, Copy, Mail, MapPinned, Timer, Utensils } from 'lucide-react'
import { useMemo, useState } from 'react'
import { chartArgs, composeArgs, eventDraftArgs, mapArgs, recipeArgs, weatherView, widgetArgs, type ChartArgs, type RecipeArgs } from '../../core/assistant-cards.ts'
import { renderStep, scaleAmount } from '../../core/assistant-cards.ts'
import type { ChartSpec } from './g18-chart-data.ts'
import { ChartCard } from './G18ChartBlock.tsx'
import { useG3Actions } from './g3-bridge.ts'
import { Markdown } from './Markdown.tsx'
import { SandboxedPreview } from './SandboxedPreview.tsx'
import type { ToolItem } from './chat-state.ts'
import { percentText } from './ui-locale.ts'

function parse<T>(schema: { safeParse(v: unknown): { success: true; data: T } | { success: false } }, raw: string): T | null {
  try {
    const parsed = schema.safeParse(JSON.parse(raw))
    return parsed.success ? parsed.data : null
  } catch {
    return null
  }
}

function toSpec(args: ChartArgs): ChartSpec {
  const rows = args.series.map((s) => s.data ?? s.values ?? [])
  const count = Math.max(...rows.map((r) => r.length), 0)
  const labels = (args.x_axis?.categories ?? []).slice(0, count)
  while (labels.length < count) labels.push(String(labels.length + 1))
  return {
    type: args.style,
    ...(args.title ? { title: args.title } : {}),
    labels,
    series: args.series.map((s) => ({
      name: s.name,
      values: s.data ?? s.values ?? [],
      axis: 'left' as const,
      ...(s.color ? { color: s.color } : {}),
    })),
    stacked: false,
    ...(args.x_axis?.label ? { xLabel: args.x_axis.label } : {}),
    ...(args.y_axis?.label ? { yLabel: args.y_axis.label } : {}),
    refLines: [],
  }
}

export function AssistantCard({ tool, waiting }: { tool: ToolItem; waiting: boolean }) {
  switch (tool.name) {
    case 'compose_message': {
      const args = parse(composeArgs, tool.summary)
      return args ? <ComposeCard args={args} /> : null
    }
    case 'show_chart': {
      const args = parse(chartArgs, tool.summary)
      return args ? (
        <div className="message">
          <ChartCard spec={toSpec(args)} source={tool.summary} />
        </div>
      ) : null
    }
    case 'show_widget': {
      const args = parse(widgetArgs, tool.summary)
      return args ? <WidgetCard title={args.title} code={args.widget_code} note={args.loading_messages?.[0]} /> : null
    }
    case 'show_recipe': {
      const args = parse(recipeArgs, tool.summary)
      return args ? <RecipeCard args={args} /> : null
    }
    case 'weather': {
      const view = tool.result ? parse(weatherView, tool.result) : null
      return view ? <WeatherCard view={view} /> : null
    }
    case 'show_map': {
      const args = parse(mapArgs, tool.summary)
      return args ? <MapCard title={args.title} markers={args.markers} /> : null
    }
    case 'create_event': {
      const args = parse(eventDraftArgs, tool.summary)
      return args ? <DraftCard tool={tool} title={args.title} when={args.start_time} place={args.location} waiting={waiting} /> : null
    }
    default:
      return null
  }
}

function ComposeCard({ args }: { args: { summary_title: string; kind: string; variants: { label: string; body: string; subject?: string }[] } }) {
  const [index, setIndex] = useState(0)
  const [copied, setCopied] = useState(false)
  const variant = args.variants[index] ?? args.variants[0]
  if (!variant) return null
  const text = variant.subject ? `Konu: ${variant.subject}\n\n${variant.body}` : variant.body
  const copy = () => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }
  const mailto = args.kind === 'email' ? `mailto:?subject=${encodeURIComponent(variant.subject ?? args.summary_title)}&body=${encodeURIComponent(variant.body)}` : null
  return (
    <div className="message">
      <section className="g3-card g23-card" aria-label={args.summary_title}>
        <header className="g3-card-head">
          <Mail size={14} aria-hidden="true" />
          <span>{args.summary_title}</span>
        </header>
        {args.variants.length > 1 && (
          <div className="g23-tabs" role="tablist">
            {args.variants.map((v, i) => (
              <button key={v.label} type="button" className="g23-tab" role="tab" aria-selected={i === index} onClick={() => setIndex(i)}>
                {v.label}
              </button>
            ))}
          </div>
        )}
        {variant.subject && <p className="g23-body">{variant.subject}</p>}
        <p className="g23-body">{variant.body}</p>
        <div className="g23-row">
          <button type="button" className="btn small" onClick={copy}>
            {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
            {copied ? 'Kopyalandı' : 'Kopyala'}
          </button>
          {mailto && (
            <a className="btn small primary" href={mailto}>
              Posta uygulamasında aç
            </a>
          )}
        </div>
      </section>
    </div>
  )
}

function WidgetCard({ title, code, note }: { title: string; code: string; note?: string }) {
  const svg = code.trim().startsWith('<svg')
  const html = code.trim().startsWith('<') ? code : `<pre>${code.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c] ?? c)}</pre>`
  return (
    <div className="message">
      <section className="g3-card g23-card" aria-label={title}>
        <header className="g3-card-head">
          <span>{title}</span>
        </header>
        {note && <p className="muted">{note}</p>}
        <SandboxedPreview content={svg ? code : html} kind={svg ? 'svg' : 'html'} title={title} allowScriptsToggle={!svg} height={280} />
      </section>
    </div>
  )
}

function RecipeCard({ args }: { args: RecipeArgs }) {
  const base = args.base_servings ?? 4
  const [servings, setServings] = useState(base)
  const [step, setStep] = useState(0)
  const [left, setLeft] = useState<number | null>(null)
  const current = args.steps[step]
  const start = (seconds: number) => {
    setLeft(seconds)
    const tick = window.setInterval(() => {
      setLeft((n) => {
        if (n === null || n <= 1) {
          window.clearInterval(tick)
          return 0
        }
        return n - 1
      })
    }, 1000)
  }
  return (
    <div className="message">
      <section className="g3-card g23-card" aria-label={args.title}>
        <header className="g3-card-head">
          <Utensils size={14} aria-hidden="true" />
          <span>{args.title}</span>
        </header>
        {args.description && <p className="muted">{args.description}</p>}
        <div className="g23-row">
          <span>Porsiyon</span>
          <button type="button" className="btn small" onClick={() => setServings((n) => Math.max(1, n - 1))} aria-label="Porsiyon azalt">
            −
          </button>
          <span>{servings}</span>
          <button type="button" className="btn small" onClick={() => setServings((n) => Math.min(100, n + 1))} aria-label="Porsiyon artır">
            +
          </button>
        </div>
        <ul>
          {args.ingredients.map((item) => (
            <li key={item.id}>
              {scaleAmount(item.amount, base, servings)}
              {item.unit ? ` ${item.unit}` : ''} {item.name}
            </li>
          ))}
        </ul>
        {current && (
          <div className="g23-step">
            <strong>
              {step + 1}/{args.steps.length} {current.title}
            </strong>
            <p>{renderStep(current.content, args.ingredients, base, servings)}</p>
            <div className="g23-row">
              <button type="button" className="btn small" disabled={step === 0} onClick={() => setStep((n) => n - 1)}>
                <ChevronLeft size={14} aria-hidden="true" /> Önceki
              </button>
              <button type="button" className="btn small" disabled={step >= args.steps.length - 1} onClick={() => setStep((n) => n + 1)}>
                Sonraki <ChevronRight size={14} aria-hidden="true" />
              </button>
              {current.timer_seconds && (
                <button type="button" className="btn small" onClick={() => start(current.timer_seconds ?? 0)}>
                  <Timer size={14} aria-hidden="true" /> {left === null ? 'Sayacı başlat' : left === 0 ? 'Bitti' : `${left} sn`}
                </button>
              )}
            </div>
          </div>
        )}
        {args.notes && <p className="muted">{args.notes}</p>}
      </section>
    </div>
  )
}

/** Arka uç WMO durumlarını sabit İngilizce kodla verir; kart Türkçe kaynak metni gösterir, çeviri katmanı dile çevirir. */
const WEATHER_TEXT: Record<string, string> = {
  Clear: 'Açık hava',
  'Mainly clear': 'Çoğunlukla açık',
  'Partly cloudy': 'Parçalı bulutlu',
  Overcast: 'Kapalı hava',
  Fog: 'Sisli',
  Drizzle: 'Çiseleyen yağmur',
  Rain: 'Yağmurlu',
  'Freezing rain': 'Dondurucu yağmur',
  Snow: 'Karlı',
  'Rain showers': 'Sağanak yağmur',
  'Snow showers': 'Kar sağanağı',
  Thunderstorm: 'Gök gürültülü fırtına',
  'Thunderstorm with hail': 'Dolulu fırtına',
  Unknown: 'Bilinmiyor',
}

export function weatherText(label: string): string {
  return WEATHER_TEXT[label] ?? label
}

function WeatherCard({ view }: { view: { location_name: string; current: { temp_c: number; label: string; wind_kmh: number; precipitation_mm: number }; days: { date: string; label: string; max_c: number; min_c: number; precip_probability: number }[] } }) {
  return (
    <div className="message">
      <section className="g3-card g23-card" aria-label={`Hava: ${view.location_name}`}>
        <header className="g3-card-head">
          <span>{view.location_name}</span>
        </header>
        <div className="g23-weather-now">
          <span className="g23-temp">{Math.round(view.current.temp_c)}°</span>
          <span>{weatherText(view.current.label)}</span>
        </div>
        <p className="muted">
          Rüzgar {Math.round(view.current.wind_kmh)} km/s · Yağış {view.current.precipitation_mm} mm
        </p>
        <ul className="g23-days">
          {view.days.map((d) => (
            <li key={d.date} className="g23-day">
              <span>{d.date.slice(5)}</span>
              <span>{weatherText(d.label)}</span>
              <span>
                {Math.round(d.max_c)}° / {Math.round(d.min_c)}°
              </span>
              <span className="muted">{percentText(Math.round(d.precip_probability))}</span>
            </li>
          ))}
        </ul>
        <p className="muted">Veri: Open-Meteo</p>
      </section>
    </div>
  )
}

function MapCard({ title, markers }: { title?: string; markers: { lat: number; lon: number; label?: string }[] }) {
  const source = useMemo(() => JSON.stringify({ title, markers }), [title, markers])
  return (
    <div className="message">
      <section className="g3-card g23-card" aria-label={title ?? 'Harita'}>
        <header className="g3-card-head">
          <MapPinned size={14} aria-hidden="true" />
          <span>{title ?? 'Harita'}</span>
        </header>
        <Markdown text={'```map\n' + source + '\n```'} />
      </section>
    </div>
  )
}

/** "2026-11-11T15:00" → "11 Kas 2026 15:00" (yalnız tarih: saatsiz); çözülemezse olduğu gibi. */
export function readableTime(iso: string): string {
  const dateOnly = /^\d{4}-\d{2}-\d{2}$/.test(iso.trim())
  // Yalnız tarih UTC gece yarısı sayılmasın: yerel gün başı.
  const d = new Date(dateOnly ? `${iso.trim()}T00:00` : iso)
  if (Number.isNaN(d.getTime())) return iso
  return d.toLocaleString(document.documentElement.lang || undefined, {
    dateStyle: 'medium',
    ...(dateOnly ? {} : { timeStyle: 'short' }),
  })
}

function DraftCard({ tool, title, when, place, waiting }: { tool: ToolItem; title: string; when: string; place?: string; waiting: boolean }) {
  const getActions = useG3Actions()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const answer = (confirmed: boolean) => {
    const actions = getActions()
    if (!actions) return
    setBusy(true)
    void actions
      .answer(tool.id, { kind: 'draft', confirmed })
      .catch((err: unknown) => setError(err instanceof Error ? err.message : 'Onay gönderilemedi'))
      .finally(() => setBusy(false))
  }
  return (
    <div className="message">
      <section className="g3-card g23-card" aria-label="Etkinlik taslağı">
        <header className="g3-card-head">
          <span>{waiting ? 'Etkinlik onayı' : 'Etkinlik'}</span>
        </header>
        <p>
          <strong>{title}</strong>
        </p>
        <p>{readableTime(when)}</p>
        {place && <p>{place}</p>}
        {waiting && (
          <div className="g23-row">
            <button type="button" className="btn small primary" disabled={busy} onClick={() => answer(true)}>
              Kaydet
            </button>
            <button type="button" className="btn small" disabled={busy} onClick={() => answer(false)}>
              Vazgeç
            </button>
          </div>
        )}
        {!waiting && tool.result && <p className="g3-answer">{tool.result}</p>}
        {error && (
          <p className="error-text" role="alert">
            {error}
          </p>
        )}
      </section>
    </div>
  )
}
