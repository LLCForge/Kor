// §27.2 bildirim merkezi: başlık çubuğunda zil + okunmamış rozeti, sağdan açılan panel (filtre, okundu,
// tümünü temizle, 30 gün). `kor notifications` ile aynı depo.
import { Bell, BellOff, BellRing, Check, CheckCheck, FileDiff, MessageSquare, Trash2, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { KorNotification } from '../../ipc-shellui.ts'
import { errorMessage } from '../api.ts'
import { shellui, useNotifications, useShellState } from './bridge.ts'
import { CATEGORY_LABEL, filterNotifications, presentCategories, usd, type NotificationFilter } from './state.ts'
import { usePresence } from '../use-presence.ts'
import { openView } from '../view-catalog.ts'

/** Palet ve tepsi "Bildirimler"i açar. */
export const OPEN_BELL_EVENT = 'kor:bell-open'

function when(at: number): string {
  const d = new Date(at)
  return d.toLocaleString(undefined, { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' })
}

interface ItemProps {
  n: KorNotification
  pending: boolean
  onOpen: (n: KorNotification, panel?: string) => void
  onChanged: () => void
}

function Item({ n, pending, onOpen, onChanged }: ItemProps) {
  const [problem, setProblem] = useState<string | null>(null)
  const act = async (fn: () => Promise<unknown>) => {
    setProblem(null)
    try {
      await fn()
      onChanged()
    } catch (err) {
      setProblem(errorMessage(err))
    }
  }
  return (
    <li className={`ksh-note ${n.read ? 'read' : 'unread'} cat-${n.category}`}>
      <div className="ksh-note-head">
        <span className="ksh-chip">{CATEGORY_LABEL[n.category]}</span>
        <span className="ksh-muted">{when(n.at)}</span>
        {!n.read && (
          <button type="button" className="action-btn" aria-label="Okundu say" title="Okundu say" onClick={() => void act(() => shellui.notificationsRead([n.id]))} data-kor-cmd="shellui.notificationsRead">
            <Check size={14} aria-hidden="true" />
          </button>
        )}
        <button type="button" className="action-btn" aria-label="Bildirimi kaldır" title="Bildirimi kaldır" onClick={() => void act(() => shellui.notificationsClear([n.id]))} data-kor-cmd="shellui.notificationsClear">
          <X size={14} aria-hidden="true" />
        </button>
      </div>
      <p className="ksh-note-title">{n.title}</p>
      {n.body && (
        <p className="ksh-note-body" translate="no">
          {n.body}
        </p>
      )}
      {(n.costUsd !== null || n.files !== null) && (
        <p className="ksh-muted">
          {n.files !== null && `${n.files} dosya`}
          {n.files !== null && n.costUsd !== null && ' · '}
          {n.costUsd !== null && usd(n.costUsd, n.costEstimated)}
        </p>
      )}
      <div className="ksh-actions">
        {n.session && (
          <button type="button" className="btn small" onClick={() => onOpen(n)}>
            <MessageSquare size={14} aria-hidden="true" /> Sohbeti aç
          </button>
        )}
        {n.session && n.category === 'done' && (n.files ?? 0) > 0 && (
          <button type="button" className="btn small" onClick={() => onOpen(n, 'changes')}>
            <FileDiff size={14} aria-hidden="true" /> Diff'i gör
          </button>
        )}
        {n.approval && pending && (
          <>
            <button type="button" className="btn small" onClick={() => void act(() => shellui.deny(n.approval ?? ''))} data-kor-cmd="shellui.deny">
              <X size={14} aria-hidden="true" /> Reddet
            </button>
            <button type="button" className="btn small primary" onClick={() => void act(() => shellui.approve(n.approval ?? ''))} data-kor-cmd="shellui.approve">
              <Check size={14} aria-hidden="true" /> Onayla
            </button>
          </>
        )}
        {n.category === 'update' && (
          <button type="button" className="btn small" onClick={() => onOpen(n, 'update')}>
            Ayrıntılar
          </button>
        )}
      </div>
      {problem && (
        <p className="error-text" role="alert">
          {problem}
        </p>
      )}
    </li>
  )
}

interface Props {
  /** Bildirimin sohbetini (ve isteğe bağlı paneli) açar. */
  onOpen: (sessionId: string | null, panel?: string) => void
}

/** Zil düğmesi + açılır panel. */
export function NotificationCenter({ onOpen }: Props) {
  const [open, setOpen] = useState(false)
  const [filter, setFilter] = useState<NotificationFilter>('all')
  const [pulse, setPulse] = useState(false)
  const { data, error, isLoading, mutate } = useNotifications()
  const { data: shell } = useShellState()
  const pending = new Set((shell?.overview.approvals ?? []).map((a) => a.id))

  useEffect(() => {
    const show = () => setOpen(true)
    window.addEventListener(OPEN_BELL_EVENT, show)
    return () => window.removeEventListener(OPEN_BELL_EVENT, show)
  }, [])
  // §28.4: pencere öndeyken ve kullanıcı etkinken yalnızca sessiz satır içi vurgu.
  useEffect(
    () =>
      shellui.onNotification(({ alert }) => {
        if (!alert.inline) return
        setPulse(true)
        window.setTimeout(() => setPulse(false), 1600)
      }),
    [],
  )
  useEffect(() => {
    if (!open) return
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  // Çekmece kapanırken `--kor-dur-panel` boyunca sağa kayarak çıkar (§23.1).
  const drawerShown = usePresence(open, 'panel')
  const items = data?.items ?? []
  const unread = data?.unread ?? 0
  const shown = filterNotifications(items, filter)
  const openItem = (n: KorNotification, panel?: string) => {
    if (!n.read) void shellui.notificationsRead([n.id]).then(() => mutate())
    setOpen(false)
    onOpen(n.session, panel)
  }
  const label = unread > 0 ? `Bildirimler (${unread} okunmamış)` : 'Bildirimler'
  return (
    <>
      <button
        type="button"
        className={`titlebar-btn ksh-bell${pulse ? ' pulse' : ''}`}
        aria-label={label}
        title={label}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        data-kor-cmd="shellui.notifications"
        data-kor-event="kor:shellui/notification"
      >
        <Bell size={14} aria-hidden="true" />
        {unread > 0 && <span className="ksh-badge">{unread > 99 ? '99+' : unread}</span>}
      </button>
      {drawerShown && (
        <aside className={`ksh-drawer${open ? '' : ' closing'}`} inert={!open} role="dialog" aria-label="Bildirim merkezi">
          <header className="ksh-drawer-head">
            <h2>Bildirimler</h2>
            <button
              type="button"
              className="action-btn"
              aria-label="Onaylar ve bildirimler panelini aç"
              title="Onaylar ve bildirimler panelini aç"
              onClick={() => {
                setOpen(false)
                openView('shellui')
              }}
            >
              <BellRing size={14} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="action-btn"
              aria-label={shell?.overview.paused ? 'Bildirimleri sürdür' : 'Bildirimleri duraklat'}
              title={shell?.overview.paused ? 'Bildirimleri sürdür' : 'Bildirimleri duraklat'}
              aria-pressed={shell?.overview.paused ?? false}
              onClick={() => void shellui.pause(!(shell?.overview.paused ?? false))}
              data-kor-cmd="shellui.pause"
            >
              {shell?.overview.paused ? <BellOff size={14} aria-hidden="true" /> : <Bell size={14} aria-hidden="true" />}
            </button>
            <button type="button" className="action-btn" aria-label="Tümünü okundu say" title="Tümünü okundu say" disabled={unread === 0} onClick={() => void shellui.notificationsRead(null).then(() => mutate())} data-kor-cmd="shellui.notificationsRead">
              <CheckCheck size={14} aria-hidden="true" />
            </button>
            <button type="button" className="action-btn" aria-label="Tümünü temizle" title="Tümünü temizle" disabled={items.length === 0} onClick={() => void shellui.notificationsClear(null).then(() => mutate())} data-kor-cmd="shellui.notificationsClear">
              <Trash2 size={14} aria-hidden="true" />
            </button>
            <button type="button" className="action-btn" aria-label="Kapat" title="Kapat" onClick={() => setOpen(false)}>
              <X size={14} aria-hidden="true" />
            </button>
          </header>
          <div className="ksh-filters" role="group" aria-label="Filtre">
            {(['all', 'unread', ...presentCategories(items)] as NotificationFilter[]).map((f) => (
              <button key={f} type="button" className={`ksh-chip-btn${filter === f ? ' active' : ''}`} aria-pressed={filter === f} onClick={() => setFilter(f)}>
                {f === 'all' ? 'Tümü' : f === 'unread' ? 'Okunmamış' : CATEGORY_LABEL[f]}
              </button>
            ))}
          </div>
          {isLoading && <p className="ksh-muted" role="status">Yükleniyor…</p>}
          {error && (
            <p className="error-text" role="alert">
              Bildirimler alınamadı: {errorMessage(error)}
            </p>
          )}
          {!isLoading && !error && shown.length === 0 && (
            <div className="ksh-empty">
              <Bell size={20} aria-hidden="true" />
              <p>{filter === 'all' ? 'Bildirim yok.' : 'Bu filtrede bildirim yok.'}</p>
              <p className="ksh-muted">Biten görevler, bekleyen onaylar, bütçe ve güncelleme uyarıları 30 gün burada kalır.</p>
            </div>
          )}
          <ul className="ksh-notes">
            {shown.map((n) => (
              <Item key={n.id} n={n} pending={n.approval !== null && pending.has(n.approval)} onOpen={openItem} onChanged={() => void mutate()} />
            ))}
          </ul>
        </aside>
      )}
    </>
  )
}
