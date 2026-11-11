// Başlık çubuğunun sağı (§27.4, §27.2, §28.5): "Güncelle · vX" pill'i, onay kuyruğu düğmesi, zil.
import { Download, ListChecks, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { G16PackagingApi } from '../../ipc-g16.ts'
import { api, errorMessage } from '../api.ts'
import { openG16Panel } from '../g16-bus.ts'
import { ApprovalQueue } from './ApprovalQueue.tsx'
import { OPEN_TARGET_EVENT, shellui, useShellState } from './bridge.ts'
import { NotificationCenter } from './NotificationCenter.tsx'
import { usePresence } from '../use-presence.ts'
import { updatePillLabel } from './state.ts'

/** Palet ve `kor:shellui/openPanel` "approvals" kuyruğu açar. */
export const OPEN_QUEUE_EVENT = 'kor:queue-open'

/** §27.4: yeni sürüm varsa ember pill; yoksa hiçbir şey. Aynı kurulum akışı (g16) kullanılır. */
function UpdatePill({ version }: { version: string }) {
  const [open, setOpen] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const shown = usePresence(open)
  const run = async (fn: () => Promise<unknown>, done: string) => {
    setBusy(true)
    setNote(null)
    try {
      await fn()
      setNote(done)
    } catch (err) {
      setNote(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <span className="ksh-pill-wrap">
      <button type="button" className="ksh-pill" aria-expanded={open} onClick={() => setOpen((o) => !o)} data-kor-event="kor:g16/updateAvailable">
        <Download size={12} aria-hidden="true" />
        <span translate="no">{updatePillLabel(version)}</span>
      </button>
      {shown && (
        <div className={`ksh-popover${open ? '' : ' closing'}`} role="dialog" aria-label="Güncelleme" inert={!open}>
          <p>
            Kor <span translate="no">{version}</span> hazır.
          </p>
          <div className="ksh-actions">
            <button type="button" className="btn small" onClick={() => { setOpen(false); openG16Panel() }}>
              Sürüm notları
            </button>
            <button type="button" className="btn small primary" disabled={busy} onClick={() => void run(() => api.g16UpdateInstall(), 'İndiriliyor; hazır olunca yeniden başlatıp kurar.')} data-kor-cmd="g16UpdateInstall">
              Şimdi güncelle
            </button>
            <button type="button" className="btn small" disabled={busy} onClick={() => void run(() => shellui.updateOnExit(), 'Kor kapanırken kurulacak.')} data-kor-cmd="shellui.updateOnExit">
              Kapanışta güncelle
            </button>
          </div>
          {note && <p className="ksh-muted" role="status">{note}</p>}
        </div>
      )}
    </span>
  )
}

/** Onay kuyruğu düğmesi (bekleyen sayısıyla) ve açılır kuyruk. */
function QueueButton({ count }: { count: number }) {
  const [open, setOpen] = useState(false)
  const shown = usePresence(open)
  useEffect(() => {
    const show = () => setOpen(true)
    window.addEventListener(OPEN_QUEUE_EVENT, show)
    return () => window.removeEventListener(OPEN_QUEUE_EVENT, show)
  }, [])
  if (count === 0 && !shown) return null
  const label = `Onay kuyruğu (${count})`
  return (
    <span className="ksh-pill-wrap">
      <button type="button" className="titlebar-btn ksh-queue-btn" aria-label={label} title={label} aria-expanded={open} onClick={() => setOpen((o) => !o)} data-kor-cmd="shellui.queue">
        <ListChecks size={14} aria-hidden="true" />
        {count > 0 && <span className="ksh-badge warn">{count}</span>}
      </button>
      {shown && (
        <div className={`ksh-popover wide${open ? '' : ' closing'}`} role="dialog" aria-label="Onay kuyruğu" inert={!open}>
          <div className="ksh-popover-head">
            <strong>Onay kuyruğu</strong>
            <button type="button" className="action-btn" aria-label="Kapat" title="Kapat" onClick={() => setOpen(false)}>
              <X size={14} aria-hidden="true" />
            </button>
          </div>
          <ApprovalQueue autoFocus onClose={() => setOpen(false)} />
        </div>
      )}
    </span>
  )
}

const openTarget = (sessionId: string | null, panel?: string) =>
  window.dispatchEvent(new CustomEvent(OPEN_TARGET_EVENT, { detail: { sessionId, panel } }))

export function TitleItems() {
  const { data } = useShellState()
  const [found, setFound] = useState<string | null>(null)
  // Pill hem açılıştaki durumdan hem `kor:g16/updateAvailable` olayından beslenir.
  useEffect(() => (api as unknown as G16PackagingApi).g16.onUpdateAvailable((e) => setFound(e.version)), [])
  const version = found ?? data?.update ?? null
  return (
    <div className="ksh-title-items">
      {version && <UpdatePill version={version} />}
      <QueueButton count={data?.overview.approvals.length ?? 0} />
      <NotificationCenter onOpen={openTarget} />
    </div>
  )
}
