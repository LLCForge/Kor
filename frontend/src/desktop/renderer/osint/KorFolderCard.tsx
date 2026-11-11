// `~/Kor/` klasörü (§27.5, ADR 0052): aç/kapat (onayla), şimdi eşitle, dosya yöneticisinde aç, sanal sürücü
// (`kor mount`, WebDAV) bağla/ayır. Canlı geçişler `kor:korfolder/changed` ile gelir.
import { FolderOpen, FolderSync, HardDrive, RefreshCw } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { KorFolderStatus } from '../../ipc-korfolder.ts'
import { errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { korfolderApi } from './batch-bridge.ts'
import { passSummary } from './batch-shape.ts'
import '../panels/graph/panel.css'
import '../panels/security/panel.css'
import './korfolder.css'

const KEY = 'korfolder:status'

const FOLDER_INFO: ReadonlyArray<{ name: string; desc: string }> = [
  { name: 'Oturumlar', desc: 'her sohbet Markdown olarak; başlık, sabitleme ve arşiv düzenlenebilir' },
  { name: 'Görseller', desc: 'projelerin kor-media klasörleri ve Galeri.md' },
  { name: 'Raporlar', desc: 'perf, bench ve diğer rapor klasörleri' },
  { name: 'Diffler', desc: 'geri alınabilir değişikliklerin farkları' },
  { name: 'Kayıtlar', desc: 'oturum kayıtları (replay)' },
  { name: 'Gelen', desc: 'bırakılan dosyalar bir sonraki sohbete bağlam olur' },
  { name: 'Görevler', desc: 'bırakılan .md dosyası salt okunur bir görev başlatır' },
]

export function KorFolderCard() {
  const id = useId()
  const { data, error, isLoading, mutate } = useSWR<KorFolderStatus>(KEY, () => korfolderApi.status())
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  useEffect(
    () =>
      korfolderApi.onChanged((p) => {
        const s = passSummary(p)
        if (s) setMessage(s)
        void mutate()
      }),
    [mutate],
  )

  const run = async (fn: () => Promise<string | null>) => {
    setBusy(true)
    setActionError(null)
    try {
      setMessage(await fn())
      await mutate()
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const enable = () =>
    run(async () => {
      const ok = await confirmAction({
        title: '~/Kor klasörü oluşturulsun mu?',
        message:
          'Ana klasörünüzde Kor adlı bir klasör açılır. Oturumlar Markdown olarak görünür, kayıt, görsel ve rapor klasörlerine bağlantılar eklenir; veriler kopyalanmaz. Kapatınca yalnızca Kor’un oluşturdukları kaldırılır.',
        confirmLabel: 'Oluştur',
      })
      if (!ok) return null
      const r = await korfolderApi.enable()
      return `${r.written.length} görünüm yazıldı, ${r.linked.length} bağlantı eklendi.`
    })

  const disable = () =>
    run(async () => {
      const ok = await confirmAction({
        title: '~/Kor klasörü kaldırılsın mı?',
        message: 'Kor’un oluşturduğu görünümler ve bağlantılar silinir. Sizin eklediğiniz ya da değiştirdiğiniz dosyalar yerinde kalır; asıl veriler etkilenmez.',
        confirmLabel: 'Kaldır',
        danger: true,
      })
      if (!ok) return null
      const r = await korfolderApi.disable()
      return r.kept.length ? `${r.removed.length} öğe kaldırıldı; ${r.kept.length} dosyanız yerinde bırakıldı.` : `${r.removed.length} öğe kaldırıldı.`
    })

  const sync = () =>
    run(async () => passSummary(await korfolderApi.sync()) ?? 'Her şey güncel.')

  const toggleMount = (mounted: boolean) =>
    run(async () => {
      if (mounted) {
        await korfolderApi.unmount()
        return 'Sürücü ayrıldı.'
      }
      const m = await korfolderApi.mount()
      return m ? `Sürücü bağlandı: ${m.at}` : null
    })

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:korfolder/changed">
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <FolderSync size={16} aria-hidden="true" /> ~/Kor klasörü
        </h3>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Klasör durumunu yenile" title="Yenile" data-kor-cmd="korfolder.status">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      <p className="muted ks-note">
        Kor’un oturumları, görselleri, raporları ve kayıtları dosya yöneticisinden görülebilir, aranabilir ve sürüklenebilir. Depolama yerinde kalır; bu klasör onun okunabilir görünümüdür.
      </p>
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {isLoading && <p className="muted" role="status" data-kor-cmd="korfolder.status">Okunuyor…</p>}
      {data && !data.enabled && (
        <>
          <ul className="kb-folders">
            {FOLDER_INFO.map((f) => (
              <li key={f.name}>
                <strong translate="no">{f.name}/</strong> <span className="muted">{f.desc}</span>
              </li>
            ))}
          </ul>
          <div className="ks-actions">
            <button type="button" className="btn small primary" onClick={() => void enable()} disabled={busy} data-kor-cmd="korfolder.enable">
              Klasörü oluştur
            </button>
          </div>
        </>
      )}
      {data?.enabled && (
        <>
          <dl className="ks-facts">
            <dt>Yer</dt>
            <dd translate="no">{data.path}</dd>
            <dt>Bağlı projeler</dt>
            <dd>{data.projects.length}</dd>
            <dt>Bekleyen</dt>
            <dd>
              {data.inbox} dosya (Gelen), {data.tasks} görev
            </dd>
            <dt>Sanal sürücü</dt>
            <dd translate="no">{data.mount ? data.mount.at : 'bağlı değil'}</dd>
          </dl>
          <div className="ks-actions">
            <button type="button" className="btn small" onClick={() => void run(async () => (await korfolderApi.open(), null))} disabled={busy} data-kor-cmd="korfolder.open">
              <FolderOpen size={14} aria-hidden="true" /> Aç
            </button>
            <button type="button" className="btn small" onClick={() => void sync()} disabled={busy} data-kor-cmd="korfolder.sync">
              <RefreshCw size={14} aria-hidden="true" /> Şimdi eşitle
            </button>
            <button
              type="button"
              className="btn small"
              onClick={() => void toggleMount(data.mount !== null)}
              disabled={busy}
              data-kor-cmd={data.mount ? 'korfolder.unmount' : 'korfolder.mount'}
            >
              <HardDrive size={14} aria-hidden="true" /> {data.mount ? 'Sürücüyü ayır' : 'Sürücü olarak bağla'}
            </button>
            <button type="button" className="btn small danger" onClick={() => void disable()} disabled={busy} data-kor-cmd="korfolder.disable">
              Kaldır
            </button>
          </div>
          <p className="muted ks-note">
            Görevler/ içine bırakılan .md dosyaları salt okunur (plan) modda çalışır; dosya yazmak isteyen adımlar onaylayan olmadığı için reddedilir.
          </p>
        </>
      )}
      {message && <p className="muted" role="status">{message}</p>}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}
