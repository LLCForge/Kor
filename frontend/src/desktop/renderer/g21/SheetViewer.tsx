// [g21] Tablo önizleme: XLSX ve CSV/TSV (#175), sütun tipi algılama ve biçimlendirme (#176), tablodan
//      ```chart bloğu (#177). Hücreler React ile düz metin çizilir (enjeksiyon yok); formül/makro çalıştırılmaz.
import { ArrowDown, ArrowUp, ArrowUpDown, BarChart3, MessageSquarePlus } from 'lucide-react'
import { useMemo, useState } from 'react'
import { api } from '../api.ts'
import { CopyButton } from '../CopyButton.tsx'
import { getRichBlock } from '../rich-block-registry.ts'
import { filterRows } from '../rich-data.ts'
import { DocBar, Failure, Loading, useDocQuery } from './DocChrome.tsx'
import { insertIntoDraft } from './quote.ts'
import { scanOfficePackage } from './scan.ts'
import { compareTyped, detectDelimiter, inferColumnTypes, normalizeTable, parseDateCell, parseDelimited, parseNumberCell, toChartCsv, type ColumnType } from './table.ts'
import { openWorkbook, readSheet } from './xlsx.ts'
import { readZipText } from './zip.ts'
import { uiLocale } from '../ui-locale.ts'

const PAGE_SIZES = [25, 50, 100, 250] as const
const TYPE_LABEL: Record<ColumnType, string> = { number: 'sayı', date: 'tarih', text: 'metin', empty: 'boş' }
const numberFormat = new Intl.NumberFormat(uiLocale(), { maximumFractionDigits: 6 })
const dateFormat = new Intl.DateTimeFormat(uiLocale(), { dateStyle: 'medium', timeZone: 'UTC' })
const dateTimeFormat = new Intl.DateTimeFormat(uiLocale(), { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' })

function formatCell(value: string, type: ColumnType): string {
  if (type === 'number') {
    const n = parseNumberCell(value)
    return n === null ? value : numberFormat.format(n)
  }
  if (type === 'date') {
    const t = parseDateCell(value)
    if (t === null) return value
    return t % 86_400_000 === 0 ? dateFormat.format(t) : dateTimeFormat.format(t)
  }
  return value
}

function ChartPanel({ csv }: { csv: string }) {
  // ```chart bileşeni başka bir grubun kaydıdır; yüklüyse doğrudan çizilir, değilse blok metni sunulur.
  const Chart = getRichBlock('chart')
  const block = `\`\`\`chart\n${csv}\n\`\`\``
  return (
    <div className="g21-chart">
      <div className="g21-chart-tools">
        <span className="g21-muted">```chart bloğu (ilk sütun etiket, sayısal sütunlar seri)</span>
        <CopyButton className="action-btn" label="Grafik bloğunu kopyala" getText={() => block} />
        <button type="button" className="btn small" onClick={() => insertIntoDraft(`${block}\n`)} title="Grafik bloğunu mesaj kutusuna ekle">
          <MessageSquarePlus size={14} aria-hidden="true" /> Sohbete ekle
        </button>
      </div>
      {Chart ? <Chart code={csv} language="chart" /> : <pre className="g21-code">{block}</pre>}
    </div>
  )
}

interface TableProps {
  headers: string[]
  rows: string[][]
  truncated: boolean
  label: string
}

export function DocTable({ headers, rows, truncated, label }: TableProps) {
  const types = useMemo(() => inferColumnTypes(headers, rows), [headers, rows])
  const [sort, setSort] = useState<{ col: number; dir: 1 | -1 } | null>(null)
  const [query, setQuery] = useState('')
  const [pageSize, setPageSize] = useState<number>(50)
  const [page, setPage] = useState(0)
  const [chart, setChart] = useState(false)

  const view = useMemo(() => {
    const filtered = filterRows(rows, query)
    if (!sort) return filtered
    const type = types[sort.col] ?? 'text'
    return filtered
      .map((row, i) => ({ row, i }))
      .sort((a, b) => compareTyped(a.row[sort.col] ?? '', b.row[sort.col] ?? '', type) * sort.dir || a.i - b.i)
      .map((x) => x.row)
  }, [rows, query, sort, types])
  const chartCsv = useMemo(() => (chart ? toChartCsv(headers, view, types) : null), [chart, headers, view, types])

  const pages = Math.max(1, Math.ceil(view.length / pageSize))
  const current = Math.min(page, pages - 1)
  const pageRows = view.slice(current * pageSize, (current + 1) * pageSize)
  const toggleSort = (col: number) => setSort((s) => (s?.col !== col ? { col, dir: 1 } : s.dir === 1 ? { col, dir: -1 } : null))
  const hasNumeric = types.includes('number')

  return (
    <div className="g21-viewer">
      <DocBar position={{ label: `${label} · ${view.length} satır${truncated ? ' (kesildi)' : ''} · sayfa ${current + 1} / ${pages}`, ratio: pages > 1 ? current / (pages - 1) : 1 }}>
        <input type="search" className="g8-input g21-filter" value={query} onChange={(e) => {
            setQuery(e.target.value)
            setPage(0)
          }} placeholder="Tabloda ara…" aria-label="Tabloda ara" />
        <select className="g21-select" value={pageSize} onChange={(e) => {
            setPageSize(Number(e.target.value))
            setPage(0)
          }} aria-label="Sayfa boyutu">
          {PAGE_SIZES.map((n) => (
            <option key={n} value={n}>
              {n} satır
            </option>
          ))}
        </select>
        <button type="button" className={`action-btn${chart ? ' active' : ''}`} onClick={() => setChart((c) => !c)} disabled={!hasNumeric} aria-pressed={chart} title={hasNumeric ? 'Tablodan grafik' : 'Grafik için sayısal sütun gerekli'}>
          <BarChart3 size={15} aria-hidden="true" />
        </button>
      </DocBar>
      {chartCsv && <ChartPanel csv={chartCsv} />}
      <div className="g21-scroll g21-table-scroll" tabIndex={0} aria-label={`${label} tablosu`}>
        <table className="g21-table">
          <thead>
            <tr>
              {headers.map((h, col) => (
                <th key={col} scope="col" aria-sort={sort?.col === col ? (sort.dir === 1 ? 'ascending' : 'descending') : 'none'} className={types[col] === 'number' ? 'g21-num' : undefined}>
                  <button type="button" onClick={() => toggleSort(col)} title={`${h} sütununa göre sırala`}>
                    <span>{h}</span>
                    <small className="g21-type">{TYPE_LABEL[types[col] ?? 'text']}</small>
                    {sort?.col === col ? sort.dir === 1 ? <ArrowUp size={12} aria-hidden="true" /> : <ArrowDown size={12} aria-hidden="true" /> : <ArrowUpDown size={12} aria-hidden="true" className="g21-idle" />}
                  </button>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {pageRows.map((row, r) => (
              <tr key={current * pageSize + r}>
                {headers.map((_, col) => {
                  const type = types[col] ?? 'text'
                  return (
                    <td key={col} className={type === 'number' ? 'g21-num' : undefined}>
                      {formatCell(row[col] ?? '', type)}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {pages > 1 && (
        <div className="g8-pager">
          <button type="button" className="action-btn" onClick={() => setPage(Math.max(0, current - 1))} disabled={current === 0}>
            Önceki
          </button>
          <span>
            {current + 1} / {pages}
          </span>
          <button type="button" className="action-btn" onClick={() => setPage(Math.min(pages - 1, current + 1))} disabled={current >= pages - 1}>
            Sonraki
          </button>
        </div>
      )}
    </div>
  )
}

export function CsvViewer({ path, content }: { path: string; content: string }) {
  const table = useMemo(() => {
    const { rows, truncated } = parseDelimited(content, path.toLowerCase().endsWith('.tsv') ? '\t' : detectDelimiter(content))
    return { ...normalizeTable(rows), truncated }
  }, [content, path])
  if (table.headers.length === 0) return <p className="muted pad">Tablo boş.</p>
  return <DocTable headers={table.headers} rows={table.rows} truncated={table.truncated} label={path.slice(path.lastIndexOf('/') + 1)} />
}

export function XlsxViewer({ path }: { path: string }) {
  const book = useDocQuery(['g21', 'xlsx', path], async () => {
    const wb = await openWorkbook(await api.g21ReadDoc(path))
    const rels = await Promise.all(wb.entries.filter((e) => e.name.endsWith('.rels')).slice(0, 200).map((e) => readZipText(wb.bytes, e)))
    return { wb, findings: scanOfficePackage(wb.entries.map((e) => e.name), rels) }
  })
  const [index, setIndex] = useState(0)
  const sheet = useDocQuery(book.data ? ['g21', 'xlsx-sheet', path, index] : null, async () => {
    const wb = book.data?.wb
    if (!wb) throw new Error('Çalışma kitabı yüklenmedi')
    const data = await readSheet(wb, index)
    return { ...normalizeTable(data.rows), truncated: data.truncated }
  })
  if (book.error) return <Failure error={book.error} />
  if (!book.data) return <Loading label="Çalışma kitabı açılıyor..." />
  const { wb, findings } = book.data
  return (
    <div className="g21-viewer">
      <DocBar findings={findings}>
        <div className="g21-tabs" role="tablist" aria-label="Sayfalar">
          {wb.sheets.map((s, i) => (
            <button key={s.path} type="button" role="tab" aria-selected={i === index} className={`g21-tab${s.hidden ? ' g21-hidden-sheet' : ''}`} onClick={() => setIndex(i)} title={s.hidden ? 'Gizli sayfa' : s.name}>
              {s.name}
            </button>
          ))}
        </div>
      </DocBar>
      {sheet.error ? (
        <Failure error={sheet.error} />
      ) : !sheet.data ? (
        <Loading />
      ) : sheet.data.headers.length === 0 ? (
        <p className="muted pad">Sayfa boş.</p>
      ) : (
        <DocTable key={index} headers={sheet.data.headers} rows={sheet.data.rows} truncated={sheet.data.truncated} label={wb.sheets[index]?.name ?? ''} />
      )}
    </div>
  )
}
