// Performans paneli için saf yardımcılar (node --test ile sınanır).
import type { PerfCompareRow, PerfRow, PerfRunOptions, PerfUnit } from '../../../ipc-perf.ts'
import { percentText } from '../../ui-locale.ts'

export const PROFILERS = ['samply', 'perf', 'pprof', 'node', 'py-spy', 'cprofile', 'async-profiler', 'dotnet-trace', 'sample'] as const

/** Profil toplamı / ağırlık, birimine göre okunur biçimde. */
export function weight(v: number, unit: PerfUnit): string {
  switch (unit) {
    case 'bytes':
      if (v >= 1 << 20) return `${(v / (1 << 20)).toFixed(1)} MiB`
      if (v >= 1024) return `${(v / 1024).toFixed(1)} KiB`
      return `${v} B`
    case 'microseconds':
      return `${(v / 1000).toFixed(1)} ms`
    case 'nanoseconds':
      return `${(v / 1e6).toFixed(1)} ms`
    default:
      return `${v} örnek`
  }
}

export function pct(v: number): string {
  return percentText(v.toFixed(1))
}

/** `dosya:satır`; konum yoksa boş. */
export function location(r: Pick<PerfRow, 'file' | 'line'>): string {
  if (!r.file) return ''
  return r.line ? `${r.file}:${r.line}` : r.file
}

/** Önizlemede açılabilir mi: proje içi göreli yol (mutlak yol, URL ya da sürücü harfi değil). */
export function openable(file: string | null): file is string {
  return !!file && !/^([a-z]+:|\/|\\)/i.test(file) && !file.startsWith('..')
}

/** SVG'yi <img> kaynağı yapar: görüntü olarak yüklenen SVG betik çalıştıramaz, dış kaynak yükleyemez. */
export function svgDataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

const METRIC: Record<PerfCompareRow['metric'], string> = {
  wall_ms: 'Süre (ms)',
  samples: 'Profil toplamı',
  top_share: 'En sıcak fonksiyonun payı (%)',
  rps: 'İstek/sn',
  p50_ms: 'p50 gecikme (ms)',
  p95_ms: 'p95 gecikme (ms)',
  memory: 'Ayrılan bellek',
}

export function metricLabel(r: PerfCompareRow): string {
  return r.subject ? `${METRIC[r.metric]} · ${r.subject}` : METRIC[r.metric]
}

export function num(v: number | null): string {
  if (v === null) return '—'
  return Number.isInteger(v) ? String(v) : v.toFixed(2)
}

/** Değişimin yönü: süre/gecikme/pay/bellek için düşüş, istek/sn için artış iyidir. */
export function changeTone(r: PerfCompareRow): 'better' | 'worse' | 'same' {
  if (r.changePct === null || r.changePct === 0) return 'same'
  const higherIsBetter = r.metric === 'rps'
  return (r.changePct > 0) === higherIsBetter ? 'better' : 'worse'
}

export function changeText(r: PerfCompareRow): string {
  if (r.changePct === null) return ''
  return `${r.changePct > 0 ? '+' : ''}${r.changePct.toFixed(2)}%`
}

export interface PerfForm {
  target: string
  profiler: string
  requests: string
  concurrency: string
  duration: string
  pid: string
  cmd: string
}

export const EMPTY_FORM: PerfForm = { target: '', profiler: '', requests: '200', concurrency: '10', duration: '', pid: '', cmd: '' }

export function isEndpoint(target: string): boolean {
  return /^https?:\/\//i.test(target.trim())
}

const positive = (s: string): number | undefined => {
  const n = Number(s.trim())
  return s.trim() !== '' && Number.isInteger(n) && n > 0 ? n : undefined
}

/** Form → `perf.run` seçenekleri; boş ve geçersiz alanlar gönderilmez (sınırları arka uç uygular). */
export function toOptions(f: PerfForm, baseline?: string): PerfRunOptions {
  const o: PerfRunOptions = { target: f.target.trim() }
  if (f.profiler) o.profiler = f.profiler
  const d = positive(f.duration)
  if (d) o.duration = d
  if (isEndpoint(f.target)) {
    const r = positive(f.requests)
    const c = positive(f.concurrency)
    const p = positive(f.pid)
    if (r) o.requests = r
    if (c) o.concurrency = c
    if (p) o.pid = p
    if (f.cmd.trim()) o.cmd = f.cmd.trim()
  }
  if (baseline) o.baseline = baseline
  return o
}
