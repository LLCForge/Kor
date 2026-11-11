import { FileText, Map as MapIcon, RefreshCw, X } from 'lucide-react'
import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import useSWR from 'swr'
import type { InitProjectResult, MapProgress, ProjectMapInfo } from '../ipc-g6.ts'
import { api, errorMessage } from './api.ts'
import { formatTokens, percent } from './g6-format.ts'
import { uiLocale } from './ui-locale.ts'

const STATE_LABEL: Record<ProjectMapInfo['state'], string> = { none: 'yok', fresh: 'güncel', stale: 'eski' }

const PHASE_LABEL: Record<MapProgress['phase'], string> = {
  scan: 'Dosyalar taranıyor',
  explore: 'Alanlar keşfediliyor',
  synthesize: 'Harita yazılıyor',
  write: 'Kaydediliyor',
  init: 'AGENTS.md taslağı yazılıyor',
}

/** Kenar çubuğu, ilk açılış önerisi ve diyalog aynı veriyi paylaşır; üretim sürerken saniyede bir yoklanır. */
export function useProjectMap(root: string | null) {
  const swr = useSWR(['g6:map', root], () => api.getProjectMap(), {
    revalidateOnFocus: false,
    refreshInterval: (latest) => (latest?.building ? 1000 : 0),
  })
  const { mutate } = swr
  // Tur bitince dosya yapısı değişmiş olabilir: eskime durumu yeniden hesaplanır.
  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.type === 'done') void mutate()
      }),
    [mutate],
  )
  return swr
}

function Modal({ open, onClose, labelledBy, children }: { open: boolean; onClose: () => void; labelledBy: string; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (open && !dialog.open) dialog.showModal()
    if (!open && dialog.open) dialog.close()
  }, [open])
  return (
    <dialog ref={ref} className="dialog g6-map-dialog" onClose={onClose} aria-labelledby={labelledBy}>
      {children}
    </dialog>
  )
}

function Progress({ progress }: { progress: MapProgress }) {
  const pct = progress.total > 0 ? percent(progress.done, progress.total) : 0
  return (
    <div className="usage-row" role="status">
      <div className="usage-line">
        <span>{PHASE_LABEL[progress.phase]}</span>
        {progress.total > 0 && progress.phase === 'explore' && (
          <span className="usage-meta">
            {progress.done} / {progress.total} alan
          </span>
        )}
      </div>
      <div className="bar" role="progressbar" aria-label="Harita ilerlemesi" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <span className="bar-fill g6-map-bar" style={{ width: `${Math.max(pct, 4)}%` }} />
      </div>
      {progress.current.length > 0 && <span className="g6-note">Şu an: {progress.current.join(', ')}</span>}
    </div>
  )
}

function ProjectMapDialog({ open, onClose, root }: { open: boolean; onClose: () => void; root: string }) {
  const titleId = useId()
  const { data, error, mutate } = useProjectMap(root)
  const [message, setMessage] = useState<string | null>(null)
  const [confirmOverwrite, setConfirmOverwrite] = useState(false)
  const [initBusy, setInitBusy] = useState(false)

  const run = async (action: () => Promise<void>) => {
    setMessage(null)
    try {
      await action()
    } catch (err) {
      setMessage(errorMessage(err))
    } finally {
      await mutate()
    }
  }

  const describeInit = (result: InitProjectResult): string =>
    result.status === 'exists'
      ? ''
      : `${result.path} oluşturuldu (${result.chars.toLocaleString(uiLocale())} karakter).${result.backup ? ` Önceki hali: ${result.backup}` : ''} Yeni sohbetlerde kullanılır.`

  const init = (overwrite: boolean) =>
    run(async () => {
      setConfirmOverwrite(false)
      setInitBusy(true)
      try {
        const result = await api.initProject(overwrite)
        if (result.status === 'exists') setConfirmOverwrite(true)
        else setMessage(describeInit(result))
      } finally {
        setInitBusy(false)
      }
    })

  const building = data?.building ?? null
  const busy = building !== null || initBusy

  return (
    <Modal open={open} onClose={onClose} labelledBy={titleId}>
      <div className="dialog-body">
        <h2 id={titleId} className="dialog-title">Proje haritası</h2>
        <p className="dialog-desc text-pretty">
          Kod tabanı bir kez Explore ajanlarıyla paralel okunur ve .kor/PROJECT.md olarak özetlenir. Harita her yeni sohbetin sistem
          istemine eklenir; böylece ajan projeyi her seferinde baştan okumaz. Güncelleme yalnızca değişen alanları yeniden okur.
        </p>

        {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
        {!data && !error && <p className="muted">Yükleniyor...</p>}

        {data && (
          <div className="g6-map-facts">
            <p>
              <span className="usage-title">Durum: </span>
              <span className={`g6-state g6-state-${data.state}`}>{STATE_LABEL[data.state]}</span>
              {data.generatedAt !== null && (
                <span className="muted"> · {new Date(data.generatedAt).toLocaleString(uiLocale(), { dateStyle: 'medium', timeStyle: 'short' })}</span>
              )}
            </p>
            <p className="muted">
              {data.fileCount.toLocaleString(uiLocale())} dosya{data.truncated ? ' (tarama sınırında kesildi)' : ''}
              {data.buildTokens > 0 ? ` · ilk üretim maliyeti ~${formatTokens(data.buildTokens)} token` : ''}
            </p>
            {data.state === 'stale' && data.changedAreas.length > 0 && (
              <p className="muted">Değişen alanlar: {data.changedAreas.join(', ')}</p>
            )}
          </div>
        )}

        {building && <Progress progress={building} />}
        {data?.error && !building && <p className="error-text" role="alert">Son deneme başarısız: {data.error}</p>}
        {message && <p className="g6-note" role="status">{message}</p>}

        {confirmOverwrite && (
          <div className="g6-confirm" role="alertdialog" aria-label="AGENTS.md üzerine yazılsın mı">
            <span>AGENTS.md zaten var. Üzerine yazılsın mı? Eski hali .kor/AGENTS.md.bak olarak saklanır.</span>
            <div className="dialog-actions-right">
              <button type="button" className="btn small" onClick={() => setConfirmOverwrite(false)}>Vazgeç</button>
              <button type="button" className="btn small primary" onClick={() => void init(true)}>Üzerine yaz</button>
            </div>
          </div>
        )}

        <div className="dialog-actions">
          <button type="button" className="btn ghost" onClick={() => void init(false)} disabled={busy || !data} title="/init">
            <FileText size={14} aria-hidden="true" /> AGENTS.md oluştur
          </button>
          <div className="dialog-actions-right">
            {building ? (
              <button type="button" className="btn" onClick={() => void run(() => api.cancelProjectMap())}>
                <X size={14} aria-hidden="true" /> İptal
              </button>
            ) : (
              <>
                {data && data.state !== 'none' && (
                  <button type="button" className="btn" onClick={() => void run(() => api.buildProjectMap(true))} disabled={busy}>
                    Baştan oluştur
                  </button>
                )}
                <button
                  type="button"
                  className="btn primary"
                  onClick={() => void run(() => api.buildProjectMap(false))}
                  disabled={busy || !data || data.state === 'fresh'}
                >
                  <RefreshCw size={14} aria-hidden="true" /> {data?.state === 'none' ? 'Oluştur' : 'Güncelle'}
                </button>
              </>
            )}
            <button type="button" className="btn" onClick={onClose}>Kapat</button>
          </div>
        </div>
      </div>
    </Modal>
  )
}

/** Kenar çubuğu: "Proje haritası: yok · güncel · eski" göstergesi; tıklayınca ayrıntı ve eylemler. */
export function ProjectMapStatus({ root, collapsed }: { root: string | null; collapsed: boolean }) {
  const [open, setOpen] = useState(false)
  const { data } = useProjectMap(root)
  if (!root) return null
  const state = data?.building ? 'building' : (data?.state ?? 'none')
  const label = data?.building ? `Proje haritası: ${PHASE_LABEL[data.building.phase].toLocaleLowerCase('tr')}` : `Proje haritası: ${STATE_LABEL[data?.state ?? 'none']}`
  return (
    <>
      <button
        type="button"
        className="rail-btn"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
        title={collapsed ? label : undefined}
        aria-label={collapsed ? label : undefined}
      >
        <span className="g6-map-icon">
          <MapIcon size={16} aria-hidden="true" />
          <span className={`g6-dot g6-dot-${state}`} aria-hidden="true" />
        </span>
        <span className="rail-btn-label">{label}</span>
      </button>
      {open && <ProjectMapDialog open={open} onClose={() => setOpen(false)} root={root} />}
    </>
  )
}

/** İlk açılışta (ayar "sor" iken) harita önerisi; "Şimdi değil" bu oturumda o klasör için bir daha sormaz. */
export function ProjectMapPrompt({ root }: { root: string | null }) {
  const { data, mutate } = useProjectMap(root)
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set())
  const [error, setError] = useState<string | null>(null)
  if (!root || !data || data.root !== root || data.state !== 'none' || data.building || data.autoCreate !== 'ask' || data.fileCount === 0) return null
  if (dismissed.has(root)) return null

  const start = async () => {
    setError(null)
    try {
      await api.buildProjectMap(false)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      await mutate()
    }
  }

  return (
    <section className="g6-map-prompt pop-in" aria-label="Proje haritası önerisi">
      <p className="g6-map-prompt-title">
        <MapIcon size={16} aria-hidden="true" /> Bu proje için harita oluşturulsun mu?
      </p>
      <p className="g6-note text-pretty">
        Kod tabanı bir kez okunup .kor/PROJECT.md olarak özetlenir; sonraki sohbetler projeyi baştan okumaz ve token harcamaz.
      </p>
      {(error ?? data.error) && <p className="error-text" role="alert">{error ?? data.error}</p>}
      <div className="dialog-actions-right g6-map-prompt-actions">
        <button type="button" className="btn small ghost" onClick={() => setDismissed((prev) => new Set(prev).add(root))}>
          Şimdi değil
        </button>
        <button type="button" className="btn small primary" onClick={() => void start()}>
          Oluştur
        </button>
      </div>
    </section>
  )
}
