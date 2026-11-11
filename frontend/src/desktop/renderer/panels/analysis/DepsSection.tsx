// Bağımlılıklar (§9): `kor audit --licenses`, `kor deps check [--offline]`, `kor deps list` (ADR 0046).
import { Play, ScrollText, ShieldAlert } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import type { AnalysisReport } from '../../../ipc-analysis.ts'
import type { DepPackage, DepsApi, DepsList } from '../../../ipc-deps.ts'
import { api, errorMessage } from '../../api.ts'
import { ReportView } from './ReportView.tsx'
import { registerSection } from './section-registry.ts'
import './deps.css'

const depsApi = (api as unknown as { deps: DepsApi }).deps

const CLASS_LABEL: Record<DepPackage['class'], string> = {
  permissive: 'Serbest',
  'weak-copyleft': 'Zayıf copyleft',
  'strong-copyleft': 'Güçlü copyleft',
  proprietary: 'Tescilli',
  unknown: 'Bilinmiyor',
}

function kind(p: DepPackage): string {
  if (p.dev) return 'Geliştirme'
  return p.direct ? 'Doğrudan' : 'Dolaylı'
}

function DepsSection() {
  const id = useId()
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [list, setList] = useState<DepsList | null>(null)
  const [offline, setOffline] = useState(false)
  const [onlyDirect, setOnlyDirect] = useState(true)
  const [query, setQuery] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const run = async (what: 'licenses' | 'check' | 'list') => {
    setBusy(what)
    setError(null)
    try {
      if (what === 'list') {
        setList(await depsApi.list())
        setReport(null)
      } else {
        setReport((what === 'licenses' ? await depsApi.licenses() : await depsApi.check(offline)).report)
        setList(null)
      }
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const rows = useMemo(() => {
    if (!list) return []
    const q = query.trim().toLowerCase()
    return list.packages.filter(
      (p) => (!onlyDirect || p.direct) && (!q || p.name.toLowerCase().includes(q) || (p.license ?? '').toLowerCase().includes(q)),
    )
  }, [list, query, onlyDirect])

  return (
    <>
      <p className="ks-note muted">
        Kilit dosyaları (Cargo, npm/pnpm/yarn, Poetry/uv/Pipfile/requirements, Go, Composer, NuGet, Bundler, Maven/Gradle) okunur. Lisans kilit dosyasından ya da kurulu paketin kendi manifestinden gelir, yoksa “bilinmiyor” kalır. Güvenlik açıkları osv.dev’den, sürümler paket depolarından alınır; gizlilik ve çevrimdışı modda bu sorgular yapılmaz.
      </p>
      <div className="ks-actions">
        <button type="button" className="btn small primary" disabled={busy !== null} onClick={() => void run('licenses')} data-kor-cmd="deps.licenses">
          <ScrollText size={14} aria-hidden="true" /> {busy === 'licenses' ? 'Denetleniyor…' : 'Lisans denetimi'}
        </button>
        <button type="button" className="btn small" disabled={busy !== null} onClick={() => void run('check')} data-kor-cmd="deps.check">
          <ShieldAlert size={14} aria-hidden="true" /> {busy === 'check' ? 'Kontrol ediliyor…' : 'Bağımlılık kontrolü'}
        </button>
        <label className="ks-label kd-check" htmlFor={`${id}-off`}>
          <input id={`${id}-off`} type="checkbox" checked={offline} onChange={(e) => setOffline(e.target.checked)} />
          Önbellekten (çevrimdışı)
        </label>
        <button type="button" className="btn small" disabled={busy !== null} onClick={() => void run('list')} data-kor-cmd="deps.list">
          <Play size={14} aria-hidden="true" /> {busy === 'list' ? 'Okunuyor…' : 'Paket listesi'}
        </button>
      </div>
      {busy && !report && !list && <p className="muted" role="status">Bağımlılıklar okunuyor…</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      {report && <ReportView report={report} />}
      {list && (
        <div className="kd-list">
          <div className="ks-actions">
            <label htmlFor={`${id}-q`} className="sr-only">Paket ara</label>
            <input id={`${id}-q`} type="search" placeholder="Paket ya da lisans ara" value={query} onChange={(e) => setQuery(e.target.value)} />
            <label className="ks-label kd-check" htmlFor={`${id}-d`}>
              <input id={`${id}-d`} type="checkbox" checked={onlyDirect} onChange={(e) => setOnlyDirect(e.target.checked)} />
              Yalnızca doğrudan
            </label>
            <span className="ks-label">{rows.length} / {list.packages.length} paket</span>
          </div>
          {list.errors.map(([file, err]) => (
            <p key={file} className="error-text" role="alert">
              <span translate="no">{file}</span>: {err}
            </p>
          ))}
          {list.packages.length === 0 ? (
            <p className="ks-empty">Kilit dosyası ya da bağımlılık manifesti bulunamadı.</p>
          ) : rows.length === 0 ? (
            <p className="ks-empty">Süzgece uyan paket yok.</p>
          ) : (
            <table className="kd-table">
              <thead>
                <tr>
                  <th scope="col">Paket</th>
                  <th scope="col">Sürüm</th>
                  <th scope="col">Tür</th>
                  <th scope="col">Lisans</th>
                  <th scope="col">Konum</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((p) => (
                  <tr key={`${p.ecosystem}|${p.name}|${p.version}|${p.file}`}>
                    <td>
                      <span translate="no">{p.name}</span> <span className="muted">{p.ecosystem}</span>
                      {p.via && p.via.length > 0 && (
                        <div className="muted kd-via">
                          Çeken: <span translate="no">{p.via.slice(0, 3).join(', ')}</span>
                        </div>
                      )}
                    </td>
                    <td translate="no">{p.version || '—'}</td>
                    <td>{kind(p)}</td>
                    <td>
                      <span translate="no">{p.license ?? '—'}</span>{' '}
                      <span className={`kd-class kd-${p.class}`}>{CLASS_LABEL[p.class]}</span>
                    </td>
                    <td translate="no" className="kd-loc">
                      {p.file}:{p.line}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
      {!report && !list && !busy && !error && <p className="ks-empty">Henüz bağımlılık analizi yapılmadı.</p>}
    </>
  )
}

registerSection({ id: 'deps', label: 'Bağımlılıklar', component: DepsSection })
