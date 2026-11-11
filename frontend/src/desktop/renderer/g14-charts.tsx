// [g14] Elle çizilmiş hafif grafikler (SVG/CSS; grafik kütüphanesi yok). Her grafik tek seri ve tek renktir;
// kimlik etiketle verilir, değerler metin olarak da yazılır, ekran okuyucu için tablo eşlik eder.
import type { SpanStatus, TimelineStep } from '../../core/metrics-g14.ts'
import { formatClock, formatMs } from './g14-format.ts'

export interface BarItem {
  key: string
  value: number
  /** Çubuğun sağındaki metin */
  text: string
  title?: string
}

/** Yatay çubuklar: büyüklük karşılaştırması (maliyet, token, çağrı sayısı). */
export function HBars({ items, label, limit = 8 }: { items: readonly BarItem[]; label: string; limit?: number }) {
  const shown = items.slice(0, limit)
  const max = Math.max(...shown.map((i) => i.value), 0)
  if (shown.length === 0) return <p className="muted ks-empty">Veri yok.</p>
  return (
    <ul className="g14-hbars" aria-label={label}>
      {shown.map((i) => (
        <li key={i.key} className="g14-hbar" title={i.title ?? `${i.key}: ${i.text}`}>
          <span className="g14-hbar-key truncate">{i.key}</span>
          <span className="g14-hbar-track" aria-hidden="true">
            <span className="g14-hbar-fill" style={{ width: `${max > 0 ? Math.max(1, (i.value / max) * 100) : 0}%` }} />
          </span>
          <span className="g14-hbar-value">{i.text}</span>
        </li>
      ))}
      {items.length > limit && <li className="muted g14-more">+{items.length - limit} diğer</li>}
    </ul>
  )
}

export interface ColumnPoint {
  label: string
  /** null: o aralıkta veri yok (çubuk çizilmez) */
  value: number | null
  title: string
}

/** Zaman serisi sütunları; boş aralık boşluk olarak kalır. */
export function Columns({ points, max, label, format }: { points: readonly ColumnPoint[]; max: number; label: string; format: (v: number) => string }) {
  const width = 320
  const height = 72
  const gap = 2
  const w = points.length > 0 ? (width - gap * (points.length - 1)) / points.length : width
  const top = Math.max(max, ...points.map((p) => p.value ?? 0), 1e-9)
  return (
    <figure className="g14-columns">
      <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label} preserveAspectRatio="none">
        <line x1={0} x2={width} y1={height} y2={height} className="g14-axis" />
        {points.map((p, i) => {
          if (p.value === null) return null
          const h = Math.max(p.value > 0 ? 2 : 0, (p.value / top) * (height - 4))
          return (
            <rect key={p.label} x={i * (w + gap)} y={height - h} width={w} height={h} rx={Math.min(4, w / 2)} className="g14-col">
              <title>{p.title}</title>
            </rect>
          )
        })}
      </svg>
      {/* Uç etiketleri SVG dışında: preserveAspectRatio="none" metni gererdi. */}
      <div className="g14-ticks" aria-hidden="true">
        <span>{points[0]?.label ?? ''}</span>
        <span>{points.at(-1)?.label ?? ''}</span>
      </div>
      <table className="sr-only">
        <caption>{label}</caption>
        <tbody>
          {points.map((p) => (
            <tr key={p.label}>
              <th scope="row">{p.label}</th>
              <td>{p.value === null ? 'veri yok' : format(p.value)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  )
}

const STEP_NAMES: Record<TimelineStep['kind'], string> = { wait: 'Bekleme', thinking: 'Düşünme', response: 'Yanıt', tool: 'Araç' }
const STATUS_NAMES: Record<SpanStatus, string> = { ok: 'başarılı', error: 'hata', cancelled: 'iptal', denied: 'reddedildi' }

export const statusName = (s: SpanStatus) => STATUS_NAMES[s]

/** Tur adım zaman çizelgesi: her adım kendi satırında; adım türü etiketle, hata metinle belirtilir. */
export function Timeline({ steps, start, end }: { steps: readonly TimelineStep[]; start: number; end: number }) {
  const total = Math.max(1, end - start)
  const rowH = 18
  const labelW = 150
  const width = 640
  const plotW = width - labelW - 70
  if (steps.length === 0) return <p className="muted ks-empty">Bu turda kayıtlı adım yok.</p>
  return (
    <figure className="g14-timeline">
      <svg viewBox={`0 0 ${width} ${steps.length * rowH + 4}`} role="img" aria-label="Tur adım zaman çizelgesi">
        {steps.map((s, i) => {
          const x = labelW + ((s.start - start) / total) * plotW
          const w = Math.max(2, ((s.end - s.start) / total) * plotW)
          const y = i * rowH + 2
          const failed = s.status !== 'ok'
          return (
            <g key={`${s.kind}-${s.start}-${i}`}>
              <text x={0} y={y + 12} className="g14-tl-label">
                {s.kind === 'tool' ? s.label : STEP_NAMES[s.kind]}
                {failed ? ` (${STATUS_NAMES[s.status]})` : ''}
              </text>
              <rect x={x} y={y + 3} width={w} height={rowH - 8} rx={4} className={`g14-tl-bar ${s.kind}${failed ? ' failed' : ''}`}>
                <title>{`${STEP_NAMES[s.kind]}${s.kind === 'tool' ? `: ${s.label}` : ''} — ${formatClock(s.start)}, ${formatMs(s.end - s.start)}${failed ? `, ${STATUS_NAMES[s.status]}` : ''}`}</title>
              </rect>
              <text x={Math.min(x + w + 4, width - 66)} y={y + 12} className="g14-tl-dur">{formatMs(s.end - s.start)}</text>
            </g>
          )
        })}
      </svg>
      <table className="sr-only">
        <caption>Tur adımları</caption>
        <tbody>
          {steps.map((s, i) => (
            <tr key={`${s.start}-${i}`}>
              <th scope="row">{s.kind === 'tool' ? `Araç ${s.label}` : STEP_NAMES[s.kind]}</th>
              <td>{formatMs(s.start - start)} sonra başladı</td>
              <td>{formatMs(s.end - s.start)}</td>
              <td>{STATUS_NAMES[s.status]}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  )
}
