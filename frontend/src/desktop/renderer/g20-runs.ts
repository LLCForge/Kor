// [g20] Çalıştırma kayıtları: JS/TS (izole worker), Python ve SQL sonuçlarını tek kart modeline çevirir; aynı bloğun
//      önceki çalıştırmalarını (#144) uygulama oturumu boyunca bellekte tutar (blok yeniden çizilse de kaybolmaz).
import type { G20Image, G20PythonResult } from '../../core/g20-run.ts'
import type { G20SqlResult } from '../../core/g20-sql.ts'
import type { Chunk } from './g20-ansi.ts'
import { toChartCsv, type VarInfo } from './g20-repl-core.ts'
import type { JsRunResult } from './g20-sandbox.ts'

export type RunLang = 'js' | 'ts' | 'py' | 'sql'

export interface RunRecord {
  id: number
  at: number
  status: 'ok' | 'error' | 'timeout'
  /** Yalnız süreçte (Python) anlamlı */
  exitCode?: number | null
  durationMs: number
  chunks: Chunk[]
  truncated: boolean
  result?: string
  error?: string
  vars: VarInfo[]
  images: G20Image[]
  /** REPL sonucundan üretilen ```chart girdisi */
  chartCsv?: string
  /** SQL sonucu: [başlıklar, ...satırlar] JSON (```table bloğu girdisi) */
  tableJson?: string
  note?: string
}

export type RunDraft = Omit<RunRecord, 'id' | 'at'>

export function fromJs(r: JsRunResult): RunDraft {
  let chartCsv: string | undefined
  if (r.json !== undefined) {
    try {
      chartCsv = toChartCsv(JSON.parse(r.json)) ?? undefined
    } catch (err) {
      console.warn('REPL sonucu JSON olarak okunamadı:', err)
    }
  }
  const draft: RunDraft = {
    status: r.timedOut ? 'timeout' : r.ok ? 'ok' : 'error',
    durationMs: r.durationMs,
    chunks: r.chunks,
    truncated: r.truncated,
    vars: r.vars,
    images: [],
  }
  if (r.result !== undefined) draft.result = r.result
  if (r.error !== undefined) draft.error = r.error
  if (chartCsv) draft.chartCsv = chartCsv
  if (r.hardeningGaps.length > 0) draft.note = `Uyarı: worker'da kaldırılamayan global: ${r.hardeningGaps.join(', ')} (CSP katmanı yine etkin)`
  return draft
}

export function fromPython(r: G20PythonResult): RunDraft {
  return {
    status: r.timedOut ? 'timeout' : r.exitCode === 0 ? 'ok' : 'error',
    exitCode: r.exitCode,
    durationMs: r.durationMs,
    chunks: r.chunks,
    truncated: r.truncated,
    vars: r.vars,
    images: r.images,
    note: `Yorumlayıcı: ${r.interpreter}`,
  }
}

export function fromSql(r: G20SqlResult): RunDraft {
  return {
    status: 'ok',
    durationMs: r.durationMs,
    chunks: [],
    truncated: r.truncated,
    vars: [],
    images: [],
    tableJson: JSON.stringify([r.columns, ...r.rows]),
    note: r.truncated ? `İlk ${r.rows.length} satır gösteriliyor` : `${r.rows.length} satır`,
  }
}

const MAX_RUNS_PER_BLOCK = 10
const MAX_BLOCKS = 100
const history = new Map<string, RunRecord[]>()
let nextId = 1

export function runHistory(key: string): RunRecord[] {
  return history.get(key) ?? []
}

/** Kaydı geçmişe ekler (en yeni başta); en eski bloklar sınır aşılınca atılır. */
export function recordRun(key: string, draft: RunDraft): RunRecord[] {
  const record: RunRecord = { ...draft, id: nextId++, at: Date.now() }
  const runs = [record, ...runHistory(key)].slice(0, MAX_RUNS_PER_BLOCK)
  history.delete(key)
  history.set(key, runs)
  if (history.size > MAX_BLOCKS) history.delete(history.keys().next().value ?? '')
  return runs
}
