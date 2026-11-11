// Çevrimdışı görünümü (§6.5, ADR 0034): hazır olma durumu, çevrimdışı mod anahtarı, kurulum adımları
// (`offline.init`, ilerleme `kor:offline/progress`), air-gap paketi oluşturma ve içe aktarma.
import { CheckCircle2, CircleAlert, Download, FileDown, FileUp, RefreshCw, WifiOff } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { OfflineDone, OfflineStatus } from '../../../ipc-offline.ts'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { offlineApi } from './bridge.ts'
import { age, missingText, percent, reduceSteps, size, type StepRow } from './shape.ts'
import { percentText } from '../../ui-locale.ts'

const STATUS_KEY = 'offline:status'

const RUNTIMES: ReadonlyArray<{ id: string; label: string }> = [
  { id: '', label: 'Otomatik: çalışan sunucu, yoksa llama.cpp' },
  { id: 'llamacpp', label: 'llama.cpp (taşınabilir, Kor kurar)' },
  { id: 'ollama', label: 'Ollama (sistemde kurulu olmalı)' },
  { id: 'lmstudio', label: 'LM Studio (sistemde kurulu olmalı)' },
]

const PLATFORMS: ReadonlyArray<{ id: string; label: string }> = [
  { id: '', label: 'Bu bilgisayar' },
  { id: 'win-x64', label: 'Windows x64' },
  { id: 'win-arm64', label: 'Windows ARM64' },
  { id: 'linux-x64', label: 'Linux x64' },
  { id: 'linux-arm64', label: 'Linux ARM64' },
  { id: 'macos-x64', label: 'macOS Intel' },
  { id: 'macos-arm64', label: 'macOS Apple Silicon' },
]

const STEP_LABEL: Record<StepRow['step'], string> = {
  runtime: 'Çalıştırıcı',
  model: 'Model',
  embedding: 'Embedding',
  catalog: 'Katalog',
  config: 'Ayarlar',
  bundle: 'Paket',
  import: 'İçe aktarma',
}

function StatusCard({ data, onChange }: { data: OfflineStatus; onChange: (s: OfflineStatus) => void }) {
  const id = useId()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const toggle = async (on: boolean) => {
    setBusy(true)
    setError(null)
    try {
      onChange(await offlineApi.setEnabled(on))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className={`kg-card${data.enabled ? ' ks-privacy-on' : ''}`} aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <WifiOff size={16} aria-hidden="true" />
        Çevrimdışı mod
        <span className={`ks-badge${data.enabled ? ' on' : ''}`}>{data.enabled ? 'Açık' : 'Kapalı'}</span>
      </h3>
      <label className="ks-check">
        <input type="checkbox" checked={data.enabled} disabled={busy} onChange={(e) => void toggle(e.target.checked)} data-kor-cmd="offline.setEnabled" />
        İnternet olmadan çalış: yalnızca bu bilgisayardaki model, gizlilik modu açık, güncelleme ve katalog denetimi yok
      </label>
      {busy && <p className="muted" role="status">Yerel model sunucusu hazırlanıyor…</p>}
      {data.ready ? (
        <p className="ks-ok"><CheckCircle2 size={14} aria-hidden="true" />Hazır: tam bir görev internetsiz çalışabilir.</p>
      ) : (
        <p className="ks-broken"><CircleAlert size={14} aria-hidden="true" />Henüz hazır değil: {missingText(data.missing)}.</p>
      )}
      <dl className="ks-facts">
        <dt>Çalıştırıcı</dt>
        <dd translate="no">{data.runtime || '—'} {data.endpoint}</dd>
        <dt>Model</dt>
        <dd translate="no">{data.model || '—'}{data.serving ? ' (çalışıyor)' : ''}</dd>
        <dt>llama-server</dt>
        <dd>{data.llamaServer ? 'kurulu' : 'kurulu değil'}</dd>
        <dt>Embedding modeli</dt>
        <dd>{data.embedding ? 'kurulu' : 'yok (yalnızca metin araması)'}</dd>
        <dt>Model kataloğu</dt>
        <dd>{age(data.catalogAgeSecs)}</dd>
        <dt>Çalışan sunucular</dt>
        <dd translate="no">{data.servers.length ? data.servers.map((s) => `${s.name} (${s.models.length})`).join(', ') : '—'}</dd>
      </dl>
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function ModelPicker({ data, runtime, chosen, onChange }: { data: OfflineStatus; runtime: string; chosen: string[]; onChange: (m: string[]) => void }) {
  const flip = (m: string, on: boolean) => onChange(on ? [...chosen, m] : chosen.filter((x) => x !== m))
  const served = data.servers.find((s) => s.provider === 'local-lmstudio')?.models ?? []
  const items: Array<{ id: string; label: string }> =
    runtime === 'ollama'
      ? data.ollamaModels.map((t) => ({ id: t, label: t }))
      : runtime === 'lmstudio'
        ? served.map((m) => ({ id: m, label: m }))
        : data.models.map((m) => ({
            id: m.name.replace(/^model-/, ''),
            label: `${m.name.replace(/^model-/, '')} · ${m.installed ? 'kurulu' : size(m.size)} · ${m.about}`,
          }))
  if (!items.length) return <p className="ks-empty">{runtime === 'lmstudio' ? "LM Studio'da yüklü model yok; modeli LM Studio'dan indirin." : 'Seçilebilecek model yok.'}</p>
  return (
    <fieldset className="ko-models">
      <legend>Modeller</legend>
      {items.map((it) => (
        <label key={it.id} className="ks-check">
          <input type="checkbox" checked={chosen.includes(it.id)} onChange={(e) => flip(it.id, e.target.checked)} />
          <span translate="no">{it.label}</span>
        </label>
      ))}
    </fieldset>
  )
}

function Steps({ rows }: { rows: StepRow[] }) {
  if (!rows.length) return null
  return (
    <ul className="ko-steps" aria-live="polite">
      {rows.map((r) => {
        const pct = percent(r)
        return (
          <li key={r.key} className={`ko-step${r.state === 'error' ? ' error' : ''}`}>
            <div className="ko-step-head">
              <span>{STEP_LABEL[r.step]}</span>
              <code translate="no">{r.name}</code>
              <span className="muted">
                {r.state === 'done' ? 'tamam' : r.state === 'skip' ? 'zaten var' : r.state === 'error' ? 'hata' : pct != null ? percentText(pct) : '…'}
              </span>
            </div>
            {r.state === 'progress' && pct != null && <progress max={100} value={pct} aria-label={`${r.name} ilerlemesi`} />}
            {r.state === 'progress' && r.total ? <span className="muted">{size(r.done)} / {size(r.total)}</span> : null}
            {r.message && r.state === 'error' && <span className="error-text" role="alert">{r.message}</span>}
          </li>
        )
      })}
    </ul>
  )
}

function doneText(d: OfflineDone): string {
  if (!d.ok) return d.error ?? 'İşlem başarısız oldu'
  const r = (d.result ?? {}) as Record<string, unknown>
  if (d.job === 'init') return `Çevrimdışı kurulum kaydedildi: ${String(r.runtime ?? '')}, ${String(r.model ?? '')}.`
  if (d.job === 'bundle') return `Paket yazıldı: ${String(r.path ?? '')} (${size(Number(r.size ?? 0))}, ${String(r.files ?? 0)} dosya).`
  const cli = r.cli ? ` Paketteki kor dosyası: ${String(r.cli)} — kurmak için "kor update --from <paket>".` : ''
  return `İçe aktarıldı: ${Array.isArray(r.components) ? r.components.join(', ') : ''}.${cli}`
}

export function OfflinePanel(_: KorPanelProps) {
  const id = useId()
  const { data, error, mutate, isLoading } = useSWR(STATUS_KEY, () => offlineApi.status(), { revalidateOnFocus: false })
  const [runtime, setRuntime] = useState('')
  const [models, setModels] = useState<string[]>([])
  const [embedding, setEmbedding] = useState(true)
  const [platform, setPlatform] = useState('')
  const [withDesktop, setWithDesktop] = useState(false)
  const [rows, setRows] = useState<StepRow[]>([])
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<OfflineDone | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  useEffect(() => offlineApi.onProgress((e) => setRows((r) => reduceSteps(r, e))), [])
  useEffect(
    () =>
      offlineApi.onDone((d) => {
        setRunning(false)
        setResult(d)
        void mutate()
      }),
    [mutate],
  )

  const effectiveRuntime = runtime || (data?.runtime === 'local' ? 'ollama' : data?.runtime === 'local-lmstudio' ? 'lmstudio' : 'llamacpp')

  const start = async (job: () => Promise<{ started: boolean }>) => {
    setActionError(null)
    setResult(null)
    setRows([])
    try {
      const r = await job()
      setRunning(r.started)
    } catch (err) {
      setActionError(errorMessage(err))
    }
  }

  return (
    <div className="ks" data-kor-event="kor:offline/progress kor:offline/done">
      <div className="kg-card-head">
        <h2 className="ks-title">Çevrimdışı</h2>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Çevrimdışı durumunu yenile" title="Yenile" data-kor-cmd="offline.status">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading || !data ? (
        <p className="muted" role="status" data-kor-cmd="offline.status">Yerel çalıştırıcılar ve modeller denetleniyor…</p>
      ) : (
        <>
          <StatusCard data={data} onChange={(s) => void mutate({ ...s, ollamaModels: data.ollamaModels }, { revalidate: false })} />

          <section className="kg-card" aria-labelledby={`${id}-init`}>
            <h3 id={`${id}-init`} className="kg-card-title ks-title"><Download size={16} aria-hidden="true" />Hazırla</h3>
            <p className="muted ks-note">Çalıştırıcı, seçtiğiniz modeller, yerel embedding modeli ve model kataloğunun kalıcı bir kopyası Kor'un kendi klasörüne indirilir; her dosyanın SHA-256 özeti doğrulanır, yarıda kalan indirme kaldığı yerden sürer.</p>
            <label htmlFor={`${id}-rt`} className="ks-label">Çalıştırıcı</label>
            <select id={`${id}-rt`} className="g7-select" value={runtime} disabled={running} onChange={(e) => { setRuntime(e.target.value); setModels([]) }}>
              {RUNTIMES.map((r) => <option key={r.id} value={r.id}>{r.label}</option>)}
            </select>
            {effectiveRuntime === 'ollama' && !data.servers.some((s) => s.provider === 'local') && (
              <p className="ks-warn">Ollama çalışmıyor. ollama.com/download adresinden kurun (sistem kurulumu, onayınızı ister) ya da taşınabilir llama.cpp'yi seçin.</p>
            )}
            <ModelPicker data={data} runtime={effectiveRuntime} chosen={models} onChange={setModels} />
            <label className="ks-check">
              <input type="checkbox" checked={embedding} disabled={running} onChange={(e) => setEmbedding(e.target.checked)} />
              Yerel embedding modeli (anlamsal arama, yaklaşık 488 MB)
            </label>
            <div className="ko-actions">
              <button type="button" className="btn small primary" disabled={running || (!models.length && !data.model)} onClick={() => void start(() => offlineApi.init({ runtime: runtime || undefined, models, embedding, catalog: true }))} data-kor-cmd="offline.init">
                <Download size={14} aria-hidden="true" />
                {running ? 'Çalışıyor…' : 'İndir ve hazırla'}
              </button>
            </div>
          </section>

          <section className="kg-card" aria-labelledby={`${id}-bundle`}>
            <h3 id={`${id}-bundle`} className="kg-card-title ks-title"><FileDown size={16} aria-hidden="true" />Air-gap paketi</h3>
            <p className="muted ks-note">Tek bir arşiv: kor komut satırı, llama.cpp, seçilen modeller, embedding modeli, katalog ve her dosyanın SHA-256 listesi. USB ile internetsiz bir bilgisayara taşınır.</p>
            <label htmlFor={`${id}-pf`} className="ks-label">Hedef platform</label>
            <select id={`${id}-pf`} className="g7-select" value={platform} disabled={running} onChange={(e) => setPlatform(e.target.value)}>
              {PLATFORMS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
            </select>
            {effectiveRuntime !== 'llamacpp' && <p className="ks-warn">Pakete yalnızca llama.cpp için GGUF modelleri girer; yukarıdan llama.cpp seçip model işaretleyin.</p>}
            <label className="ks-check">
              <input type="checkbox" checked={withDesktop} disabled={running} onChange={(e) => setWithDesktop(e.target.checked)} />
              Bir masaüstü kurulum dosyası da ekle (seçmeniz istenir)
            </label>
            <div className="ko-actions">
              <button type="button" className="btn small" disabled={running || effectiveRuntime !== 'llamacpp' || (!models.length && !data.model.startsWith('local-llamacpp/'))} onClick={() => void start(() => offlineApi.bundle({ models, platform: platform || undefined, embedding, desktop: withDesktop }))} data-kor-cmd="offline.bundle">
                <FileDown size={14} aria-hidden="true" />
                Paket oluştur
              </button>
              <button type="button" className="btn small" disabled={running} onClick={() => void start(() => offlineApi.import())} data-kor-cmd="offline.import">
                <FileUp size={14} aria-hidden="true" />
                Paketi içe aktar
              </button>
            </div>
          </section>
        </>
      )}
      <Steps rows={rows} />
      {result && (result.ok ? <p className="ks-ok" role="status">{doneText(result)}</p> : <p className="ks-broken" role="alert">{doneText(result)}</p>)}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </div>
  )
}
