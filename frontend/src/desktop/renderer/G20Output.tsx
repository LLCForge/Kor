// [g20] Çalıştırma çıktısı parçaları: stdout/stderr renk ayrımı (#127), ANSI renkleri (#142), 1000 satırı aşan çıktıyı
//      katlama (#141), süre/çıkış kodu rozeti (#128), değişken denetçisi (#137), görsel çıktı (#143) ve REPL
//      sonucundan grafik (#136, ```chart bloğu biçimine çevrilerek). Her şey metin düğümü olarak çizilir.
import { ChartLine, Clock, Variable } from 'lucide-react'
import { useMemo, useState } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { foldChunks, parseAnsi, stripAnsi, type Chunk } from './g20-ansi.ts'
import type { VarInfo } from './g20-repl-core.ts'
import type { RunRecord } from './g20-runs.ts'
import { getRichBlock } from './rich-block-registry.ts'

export const FOLD_LINES = 1000

export function AnsiText({ text }: { text: string }) {
  const segments = useMemo(() => parseAnsi(text), [text])
  return (
    <>
      {segments.map((s, i) =>
        s.classes ? (
          <span key={i} className={s.classes}>
            {s.text}
          </span>
        ) : (
          s.text
        ),
      )}
    </>
  )
}

export function RunOutput({ chunks }: { chunks: readonly Chunk[] }) {
  const [expanded, setExpanded] = useState(false)
  const { visible, hiddenLines } = useMemo(() => (expanded ? { visible: [...chunks], hiddenLines: 0 } : foldChunks(chunks, FOLD_LINES)), [chunks, expanded])
  if (chunks.length === 0) return null
  return (
    <div className="g20-output-wrap">
      <pre className="g20-output" aria-label="Çalıştırma çıktısı">
        {visible.map((c, i) => (
          <span key={i} className={c.stream === 'err' ? 'g20-err' : 'g20-out'}>
            <AnsiText text={c.text} />
          </span>
        ))}
      </pre>
      <div className="g20-output-tools">
        {hiddenLines > 0 && (
          <button type="button" className="btn small" onClick={() => setExpanded(true)}>
            {hiddenLines} satır daha · Tümünü göster
          </button>
        )}
        <CopyButton className="action-btn" label="Çıktıyı kopyala" getText={() => stripAnsi(chunks.map((c) => c.text).join(''))} />
      </div>
    </div>
  )
}

export type RunStatus = 'ok' | 'error' | 'timeout'

export function RunBadges({ status, durationMs, exitCode, truncated }: { status: RunStatus; durationMs: number; exitCode?: number | null; truncated: boolean }) {
  const label = status === 'timeout' ? 'Zaman aşımı' : exitCode !== undefined ? `Çıkış kodu ${exitCode ?? '?'}` : status === 'ok' ? 'Başarılı' : 'Hata'
  return (
    <div className="g20-badges">
      <span className={`g20-badge ${status === 'ok' ? 'g20-badge-ok' : 'g20-badge-bad'}`}>{label}</span>
      <span className="g20-badge">
        <Clock size={12} aria-hidden="true" /> {durationMs < 1000 ? `${durationMs} ms` : `${(durationMs / 1000).toFixed(2)} sn`}
      </span>
      {truncated && <span className="g20-badge g20-badge-bad">Çıktı kısaltıldı</span>}
    </div>
  )
}

export function VarsPanel({ vars }: { vars: readonly VarInfo[] }) {
  if (vars.length === 0) return null
  return (
    <details className="g20-vars">
      <summary>
        <Variable size={14} aria-hidden="true" /> Değişkenler ({vars.length})
      </summary>
      <table>
        <thead>
          <tr>
            <th scope="col">Ad</th>
            <th scope="col">Tür</th>
            <th scope="col">Değer</th>
          </tr>
        </thead>
        <tbody>
          {vars.map((v) => (
            <tr key={v.name}>
              <td>
                <code>{v.name}</code>
              </td>
              <td>{v.type}</td>
              <td className="g20-var-preview">{v.preview}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </details>
  )
}

export function ImagesPanel({ images }: { images: readonly { name: string; dataUrl: string }[] }) {
  if (images.length === 0) return null
  return (
    <div className="g20-images">
      {images.map((img) => (
        <figure key={img.name}>
          {/* data: görsel <img> içinde betik çalıştıramaz; ana CSP img-src data: izinli. */}
          <img src={img.dataUrl} alt={`Betik çıktısı: ${img.name}`} />
          <figcaption>{img.name}</figcaption>
        </figure>
      ))}
    </div>
  )
}

/** Bir çalıştırmanın tam görünümü: rozetler, hata, akışlar, sonuç değeri, tablo, görseller, grafik ve değişkenler. */
export function RunView({ record }: { record: RunRecord }) {
  const Table = getRichBlock('table')
  return (
    <div className="g20-run">
      <RunBadges status={record.status} durationMs={record.durationMs} exitCode={record.exitCode} truncated={record.truncated} />
      {record.note && <p className="g20-note">{record.note}</p>}
      {record.error && (
        <pre className="g20-output" role="alert">
          <span className="g20-err">{record.error}</span>
        </pre>
      )}
      <RunOutput chunks={record.chunks} />
      {record.result !== undefined && (
        <pre className="g20-result" aria-label="Son ifadenin değeri">
          <span className="g20-result-mark" aria-hidden="true">
            {'=> '}
          </span>
          {record.result}
        </pre>
      )}
      {record.tableJson && (Table ? <Table code={record.tableJson} language="table" /> : <pre className="g20-output">{record.tableJson}</pre>)}
      <ImagesPanel images={record.images} />
      {record.chartCsv && <ChartFromOutput csv={record.chartCsv} />}
      <VarsPanel vars={record.vars} />
    </div>
  )
}

/** CSV'yi kayıtlı ```chart bloğuyla çizer; o blok yoksa tabloyla gösterir. Her durumda blok olarak kopyalanabilir. */
export function ChartFromOutput({ csv }: { csv: string }) {
  const [open, setOpen] = useState(false)
  const Chart = getRichBlock('chart')
  const Table = getRichBlock('csv')
  return (
    <div className="g20-chart">
      <div className="g20-chart-head">
        <button type="button" className="btn small" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
          <ChartLine size={14} aria-hidden="true" /> {open ? 'Grafiği gizle' : 'Grafik olarak göster'}
        </button>
        <CopyButton className="action-btn" label="```chart bloğu olarak kopyala" getText={() => `\`\`\`chart\n${csv}\n\`\`\``} />
      </div>
      {open && (Chart ? <Chart code={csv} language="chart" /> : Table ? <Table code={csv} language="csv" /> : <pre>{csv}</pre>)}
      {open && !Chart && <p className="g20-note">Grafik bloğu bu sürümde yüklü değil; veri tablo olarak gösteriliyor.</p>}
    </div>
  )
}
