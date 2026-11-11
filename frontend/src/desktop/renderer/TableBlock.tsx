// [g8] ```table / ```csv blokları: sıralanabilir (#96), süzülebilir/aranabilir (#97), sayfalı + sanal
//      kaydırmalı (#98), CSV/JSON dışa aktarmalı + panoya kopyalanabilir (#99) ve yerelde hücre düzenlenebilir
//      (#100) etkileşimli tablo. Tüm veri React ile metin olarak çizilir (enjeksiyon yok); ağ ve dosya yazma yok.
import { ArrowDown, ArrowUp, ArrowUpDown, Check, Pencil, Table2 } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { downloadText } from './rich-export.ts'
import { filterRows, parseTable, sortRows, tableToHtml, toCsv, toJsonRows, type SortDir, type TableData } from './rich-data.ts'
import { BlockFrame, ExportHtmlButton, RawFallback } from './rich-shared.tsx'

const ROW_HEIGHT = 33
const OVERSCAN = 8
const PAGE_SIZES = ['25', '50', '100', 'all'] as const
type PageSize = (typeof PAGE_SIZES)[number]

/** 'all' seçiliyken yalnızca görünen satırları çizen pencereleme kancası (sanal kaydırma, #98). */
function useWindow(count: number, scrollRef: React.RefObject<HTMLDivElement | null>) {
  const [range, setRange] = useState({ start: 0, end: 40 })
  const update = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    const start = Math.max(0, Math.floor(el.scrollTop / ROW_HEIGHT) - OVERSCAN)
    const end = Math.min(count, Math.ceil((el.scrollTop + el.clientHeight) / ROW_HEIGHT) + OVERSCAN)
    setRange((prev) => (prev.start === start && prev.end === end ? prev : { start, end }))
  }, [count, scrollRef])
  useEffect(update, [update])
  return { range, update }
}

function DataTable({ data, title }: { data: TableData; title: string }) {
  const [rows, setRows] = useState(data.rows)
  const [sortCol, setSortCol] = useState<number | null>(null)
  const [sortDir, setSortDir] = useState<SortDir>('asc')
  const [query, setQuery] = useState('')
  const [pageSize, setPageSize] = useState<PageSize>('50')
  const [page, setPage] = useState(0)
  const [editing, setEditing] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)

  // Yeni ayrıştırma (ör. akış tamamlanınca) gelince yerel düzenlemeler ve görünüm sıfırlanır.
  useEffect(() => {
    setRows(data.rows)
    setPage(0)
    setEditing(false)
  }, [data])

  // Düzenleme kipinde satırlar kararlı kalsın diye süzme/sıralama uygulanmaz (temel sıra); referanslar korunur,
  // böylece hücre yazımı doğru temel satıra işler. filterRows/sortRows aynı satır referanslarını döndürür.
  const view = useMemo(() => {
    if (editing) return rows
    const filtered = filterRows(rows, query)
    return sortCol === null ? filtered : sortRows(filtered, sortCol, sortDir)
  }, [rows, query, sortCol, sortDir, editing])

  const virtual = pageSize === 'all'
  const perPage = virtual ? view.length : Number(pageSize)
  const pageCount = virtual ? 1 : Math.max(1, Math.ceil(view.length / perPage))
  const clampedPage = Math.min(page, pageCount - 1)
  const pageRows = virtual ? view : view.slice(clampedPage * perPage, (clampedPage + 1) * perPage)
  const { range, update } = useWindow(pageRows.length, scrollRef)
  const windowRows = virtual ? pageRows.slice(range.start, range.end) : pageRows

  const toggleSort = (col: number) => {
    if (editing) return
    if (sortCol !== col) {
      setSortCol(col)
      setSortDir('asc')
    } else if (sortDir === 'asc') setSortDir('desc')
    else setSortCol(null)
  }

  const editCell = (rowIndex: number, col: number, value: string) => {
    setRows((prev) => prev.map((r, i) => (i === rowIndex ? r.map((c, j) => (j === col ? value : c)) : r)))
  }

  // Dışa aktarma geçerli görünümü yansıtır (düzenleme kipinde view === rows, yani düzenlenmiş temel veri).
  const current = useMemo((): TableData => ({ headers: data.headers, rows: view }), [data.headers, view])

  if (data.headers.length === 0) return <RawFallback code="(boş tablo)" note="Tablo verisi bulunamadı." />

  const ariaSort = (col: number): React.AriaAttributes['aria-sort'] => (sortCol === col ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none')

  return (
    <BlockFrame
      icon={<Table2 size={14} aria-hidden="true" />}
      label={`${title} · ${view.length} satır`}
      className="g8-table"
      tools={
        <>
          <button
            type="button"
            className={`action-btn${editing ? ' active' : ''}`}
            onClick={() => setEditing((e) => !e)}
            aria-pressed={editing}
            title={editing ? 'Düzenlemeyi bitir' : 'Hücreleri düzenle (yalnızca yerel)'}
          >
            {editing ? <Check size={15} aria-hidden="true" /> : <Pencil size={15} aria-hidden="true" />}
          </button>
          <button type="button" className="action-btn" onClick={() => downloadText(toCsv(current), 'tablo.csv', 'text/csv')} title="CSV indir">
            CSV
          </button>
          <button
            type="button"
            className="action-btn"
            onClick={() => downloadText(toJsonRows(current), 'tablo.json', 'application/json')}
            title="JSON indir"
          >
            JSON
          </button>
          <CopyButton className="action-btn" label="Tabloyu CSV olarak kopyala" getText={() => toCsv(current)} />
          <ExportHtmlButton title={title} getBody={() => tableToHtml(current)} />
        </>
      }
    >
      <div className="g8-table-bar">
        <input
          type="search"
          className="g8-input"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
            setPage(0)
          }}
          placeholder="Tabloda ara…"
          aria-label="Tabloda ara"
        />
        <label className="g8-pagesize">
          Sayfa:
          <select
            value={pageSize}
            onChange={(e) => {
              setPageSize(e.target.value as PageSize)
              setPage(0)
            }}
            aria-label="Sayfa boyutu"
          >
            <option value="25">25</option>
            <option value="50">50</option>
            <option value="100">100</option>
            <option value="all">Tümü</option>
          </select>
        </label>
      </div>

      <div ref={scrollRef} className={`g8-table-scroll${virtual ? ' g8-virtual' : ''}`} onScroll={virtual ? update : undefined}>
        <table>
          <thead>
            <tr>
              {data.headers.map((header, col) => (
                <th key={col} scope="col" aria-sort={ariaSort(col)}>
                  <button type="button" className="g8-th-btn" onClick={() => toggleSort(col)} disabled={editing} title={`${header} sütununa göre sırala`}>
                    <span className="g8-th-label">{header}</span>
                    {sortCol === col ? (
                      sortDir === 'asc' ? (
                        <ArrowUp size={13} aria-hidden="true" />
                      ) : (
                        <ArrowDown size={13} aria-hidden="true" />
                      )
                    ) : (
                      <ArrowUpDown size={13} aria-hidden="true" className="g8-th-idle" />
                    )}
                  </button>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {virtual && range.start > 0 && <tr aria-hidden="true" style={{ height: range.start * ROW_HEIGHT }} />}
            {windowRows.map((row) => {
              // Düzenleme kipinde satırlar temel sırada olduğundan dizin = rows içindeki konum.
              const rowIndex = editing ? rows.indexOf(row) : -1
              return (
                <tr key={editing ? rowIndex : row.join('\u0001')}>
                  {row.map((cell, col) => (
                    <td key={col}>
                      {editing ? (
                        <input
                          className="g8-cell-input"
                          value={cell}
                          onChange={(e) => editCell(rowIndex, col, e.target.value)}
                          aria-label={`${data.headers[col] ?? ''} düzenle`}
                        />
                      ) : (
                        cell
                      )}
                    </td>
                  ))}
                </tr>
              )
            })}
            {virtual && range.end < pageRows.length && <tr aria-hidden="true" style={{ height: (pageRows.length - range.end) * ROW_HEIGHT }} />}
          </tbody>
        </table>
      </div>

      {!virtual && pageCount > 1 && (
        <div className="g8-pager">
          <button type="button" className="action-btn" onClick={() => setPage((p) => Math.max(0, p - 1))} disabled={clampedPage === 0}>
            Önceki
          </button>
          <span>
            {clampedPage + 1} / {pageCount}
          </span>
          <button type="button" className="action-btn" onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))} disabled={clampedPage >= pageCount - 1}>
            Sonraki
          </button>
        </div>
      )}
    </BlockFrame>
  )
}

export function TableBlock({ code }: RichBlockProps) {
  const parsed = useMemo(() => {
    try {
      return { ok: true as const, data: parseTable(code) }
    } catch (err) {
      return { ok: false as const, error: err instanceof Error ? err.message : 'Tablo ayrıştırılamadı' }
    }
  }, [code])
  if (!parsed.ok) return <RawFallback code={code} note={parsed.error} />
  return <DataTable data={parsed.data} title="Tablo" />
}

registerRichBlock('table', TableBlock)
registerRichBlock('csv', TableBlock)
