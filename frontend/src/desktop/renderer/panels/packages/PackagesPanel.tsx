// Paketler görünümü (§15.15): monorepo paketleri, bağımlılıklar, değişikliğin etkilediği paketler ve daraltılmış
// komutlar (kopyalanabilir; Kor çalıştırmaz), paket başına maliyet.
import { Boxes, GitCompare, RefreshCw } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { PackagesAffected } from '../../../ipc-packages.ts'
import { errorMessage } from '../../api.ts'
import { CopyButton } from '../../CopyButton.tsx'
import type { KorPanelProps } from '../registry.ts'
import { packagesApi } from '../graph/bridge.ts'
import { uiLocale } from '../../ui-locale.ts'

const DAYS = [7, 30, 90] as const

function usd(v: number, estimated: boolean): string {
  const s = v === 0 ? '$0' : v < 0.01 ? `$${v.toFixed(4)}` : `$${v.toFixed(2)}`
  return estimated ? `≈ ${s}` : s
}

export function PackagesPanel(_: KorPanelProps) {
  const list = useSWR('packages:list', () => packagesApi.list(), { revalidateOnFocus: false })
  const [selected, setSelected] = useState<string | null>(null)

  if (list.error) {
    return (
      <div className="kp kp-empty" role="alert">
        <p>Paketler okunamadı</p>
        <p className="muted">{errorMessage(list.error)}</p>
        <button type="button" className="btn small" onClick={() => void list.mutate()} data-kor-cmd="packages.list">
          <RefreshCw size={14} aria-hidden="true" />
          Yeniden dene
        </button>
      </div>
    )
  }
  if (!list.data) return <p className="muted pad" role="status" data-kor-cmd="packages.list">Çalışma alanı taranıyor…</p>
  const { kinds, packages } = list.data
  if (packages.length === 0) {
    return (
      <div className="kp kp-empty">
        <Boxes size={28} aria-hidden="true" />
        <p>Bu proje bir monorepo değil</p>
        <p className="muted">Cargo, pnpm/yarn/npm çalışma alanları, Nx, Turborepo, Lerna, Go, Gradle, .NET çözümü ya da Bazel/Buck bildirimi bulunamadı.</p>
        <button type="button" className="btn small" onClick={() => void list.mutate()} data-kor-cmd="packages.list">
          <RefreshCw size={14} aria-hidden="true" />
          Yeniden tara
        </button>
      </div>
    )
  }

  return (
    <div className="kp">
      <div className="kp-head" data-kor-cmd="packages.list">
        <span>{`${packages.length} paket`}</span>
        <span className="muted" translate="no">{kinds.join(' · ')}</span>
        <button type="button" className="action-btn kp-push" onClick={() => void list.mutate()} aria-label="Paketleri yeniden tara" title="Yeniden tara">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>

      <ul className="kp-list" aria-label="Paketler">
        {packages.map((p) => (
          <li key={p.name}>
            <button type="button" className="kg-link" aria-current={selected === p.name} onClick={() => setSelected(selected === p.name ? null : p.name)} title={p.manifest}>
              <span translate="no">{p.name}</span>
              <span className="kg-link-meta">
                <span translate="no">{p.tool}</span>
                {` · ${p.deps.length} bağımlılık`}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {selected !== null && <Deps name={selected} onSelect={setSelected} />}
      <Affected />
      <Cost />
    </div>
  )
}

function Deps({ name, onSelect }: { name: string; onSelect: (n: string) => void }) {
  const { data, error } = useSWR(['packages:deps', name], () => packagesApi.deps(name), { revalidateOnFocus: false })
  return (
    <section className="kg-card" aria-label="Paket bağımlılıkları" data-kor-cmd="packages.deps">
      <span className="kg-card-name" translate="no">{name}</span>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted">Bağımlılıklar okunuyor…</p>
      ) : (
        <>
          <span className="muted kp-path" translate="no">{data.package.root || '.'}</span>
          <NameList title="Bağımlı olduğu paketler" names={data.deps} empty="Çalışma alanından paket kullanmıyor." onSelect={onSelect} />
          <NameList title="Onu kullanan paketler" names={data.dependents} empty="Hiçbir paket buna bağımlı değil." onSelect={onSelect} />
        </>
      )}
    </section>
  )
}

function NameList({ title, names, empty, onSelect }: { title: string; names: string[]; empty: string; onSelect: (n: string) => void }) {
  return (
    <div className="kg-links">
      <h4>{`${title} (${names.length})`}</h4>
      {names.length === 0 ? (
        <p className="muted">{empty}</p>
      ) : (
        <ul>
          {names.map((n) => (
            <li key={n}>
              <button type="button" className="kg-link" onClick={() => onSelect(n)}>
                <span translate="no">{n}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function Affected() {
  const [since, setSince] = useState('')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<PackagesAffected | null>(null)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      setResult(await packagesApi.affected(since.trim() || null))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="kg-tool" aria-label="Etkilenen paketler">
      <form className="kg-row" onSubmit={(e) => void submit(e)}>
        <label className="kg-field">
          <span>Şundan beri (--since)</span>
          <input value={since} onChange={(e) => setSince(e.target.value)} placeholder="HEAD (boş: çalışma ağacı)" spellCheck={false} maxLength={200} translate="no" />
        </label>
        <button type="submit" className="btn small" disabled={busy} data-kor-cmd="packages.affected">
          <GitCompare size={14} aria-hidden="true" />
          {busy ? 'Hesaplanıyor…' : 'Etkilenenleri bul'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {result && (
        <div className="kg-result" role="status">
          <span className="muted">{`${result.changed.length} dosya değişti, ${result.packages.length} paket etkileniyor`}</span>
          {result.packages.length > 0 && (
            <span translate="no" className="kp-names">
              {result.packages.map((p) => p.name).join(', ')}
            </span>
          )}
          {result.commands.length > 0 ? (
            <>
              <div className="kp-cmd-head">
                <h4>Daraltılmış komutlar</h4>
                <CopyButton getText={() => result.commands.join('\n')} label="Tüm komutları kopyala" />
              </div>
              <ul className="kg-plain kp-cmds">
                {result.commands.map((c) => (
                  <li key={c}>
                    <code translate="no">{c}</code>
                    <CopyButton getText={() => c} label="Komutu kopyala" />
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <p className="muted">Çalıştırılacak daraltılmış komut yok.</p>
          )}
        </div>
      )}
    </section>
  )
}

function Cost() {
  const [days, setDays] = useState<number>(30)
  const { data, error, isLoading } = useSWR(['packages:cost', days], () => packagesApi.cost(days), { revalidateOnFocus: false })
  const rows = data?.rows ?? []
  return (
    <section className="kg-tool" aria-label="Paket başına maliyet" data-kor-cmd="packages.cost">
      <div className="kg-row">
        <h4 className="kp-title">Paket başına maliyet</h4>
        <label className="kg-field kp-push">
          <span className="sr-only">Dönem</span>
          <select value={days} onChange={(e) => setDays(Number(e.target.value))} aria-label="Dönem">
            {DAYS.map((d) => (
              <option key={d} value={d}>
                {`Son ${d} gün`}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading ? (
        <p className="muted">Kullanım defteri okunuyor…</p>
      ) : rows.length === 0 ? (
        <p className="muted">Bu dönemde bu projede kayıtlı kullanım yok.</p>
      ) : (
        <table className="kp-table">
          <thead>
            <tr>
              <th scope="col">Paket</th>
              <th scope="col">İstek</th>
              <th scope="col">Token</th>
              <th scope="col">Tutar</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.model}>
                <td translate="no">{r.model === '-' ? <span className="muted">paket dışı</span> : r.model}</td>
                <td>{r.requests.toLocaleString(uiLocale())}</td>
                <td>{(r.prompt + r.completion).toLocaleString(uiLocale())}</td>
                <td title={r.unpriced > 0 ? `${r.unpriced} istek fiyatsız (tutara katılmadı)` : undefined}>
                  {usd(r.usd, r.estimated)}
                  {r.unpriced > 0 && ' *'}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="muted kp-note">Tutar, oturumun o sırada çalıştığı pakete yazılır; token bazında bölüştürme değildir. ≈ katalog fiyatıyla tahmin.</p>
    </section>
  )
}
