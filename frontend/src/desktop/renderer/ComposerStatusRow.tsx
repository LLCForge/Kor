// cdesktop mesaj kutusu üst şeridi: "N dosya değişti +X −Y" rozeti (paneli açar) · görev ilerlemesi ·
// çalışma göstergesi · "Son ⌄" oturum seçici (aynı projedeki sohbetler + Yeni sohbet).
import { Check, ChevronDown, CircleDot, ListChecks, Plus } from 'lucide-react'
import type { TodoItem } from '../../core/interactive.ts'
import type { SessionMeta } from '../ipc.ts'
import { relativeTime } from './session-meta.ts'
import { usePopover } from './use-popover.ts'

interface Props {
  files: number
  added: number
  removed: number
  todos: readonly TodoItem[] | null
  running: boolean
  sessions: readonly SessionMeta[]
  activeId: string | null
  onOpenPanel: () => void
  onOpenSession: (id: string) => void
  onNewChat: () => void
}

export function ComposerStatusRow({ files, added, removed, todos, running, sessions, activeId, onOpenPanel, onOpenSession, onNewChat }: Props) {
  const menu = usePopover<HTMLDivElement>()
  const todoMenu = usePopover<HTMLDivElement>()
  const done = todos?.filter((t) => t.status === 'completed').length ?? 0
  const total = todos?.length ?? 0
  const current = sessions.find((s) => s.id === activeId)
  const recent = sessions.slice(0, 8)
  // Gösterecek değişiklik ya da görev yokken şerit hiç çizilmez (oturum geçişi kenar çubuğunda).
  if (files === 0 && total === 0) return null
  return (
    <div className="cdx-status-row">
      <button type="button" className="cdx-files-pill" onClick={onOpenPanel} title="Değişiklikleri göster">
        {files} dosya değişti <span className="stat-add">+{added}</span> <span className="stat-del">-{removed}</span>
      </button>
      <span className="cdx-status-spacer" />
      {total > 0 && (
        <div className="popover-anchor" ref={todoMenu.ref}>
          <button type="button" className={`action-btn cdx-todo-btn${running && done < total ? ' has-dot' : ''}`} onClick={() => todoMenu.setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={todoMenu.open} title={`Görevler ${done}/${total}`} aria-label={`Görevler ${done}/${total}`}>
            <ListChecks size={15} aria-hidden="true" />
          </button>
          {todoMenu.open && (
            <div className="menu cdx-todo-menu pop-in" role="menu" aria-label="Görevler">
              <p className="menu-heading">Görevler · {done}/{total} tamamlandı{!running && done < total ? ' · yarım kaldı' : ''}</p>
              <div className="cdx-todo-bar"><span style={{ width: `${total ? (done / total) * 100 : 0}%` }} /></div>
              <ul className="cdx-todo-list">
                {todos!.map((t, i) => (
                  <li key={i} className={`cdx-todo ${t.status}`}>
                    <span className="cdx-todo-mark" aria-hidden="true">{t.status === 'completed' ? <Check size={12} /> : t.status === 'in_progress' ? <CircleDot size={12} /> : '○'}</span>
                    <span className="truncate">{t.content}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
      {/* §23.3 durum tek yerde: composer alt kenarındaki ember çizgi + sohbetteki KorStatus; burada ikinci spinner yok. */}
      <div className="popover-anchor" ref={menu.ref}>
        <button type="button" className="btn small cdx-session-switch" onClick={() => menu.setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={menu.open} title={current?.title ?? 'Oturum'}>
          Son <ChevronDown size={13} aria-hidden="true" />
        </button>
        {menu.open && (
          <div className="menu cdx-session-menu pop-in" role="menu" aria-label="Oturumlar">
            <button type="button" role="menuitem" className="menu-row" onClick={() => { menu.setOpen(false); onNewChat() }}>
              <Plus size={14} aria-hidden="true" /> Yeni sohbet
            </button>
            <p className="menu-heading">Oturumlar</p>
            {recent.length === 0 && <p className="menu-note">Bu projede kayıtlı sohbet yok.</p>}
            {recent.map((s) => (
              <button key={s.id} type="button" role="menuitemradio" aria-checked={s.id === activeId} className="menu-row" onClick={() => { menu.setOpen(false); if (s.id !== activeId) onOpenSession(s.id) }}>
                <span className="g1-check" aria-hidden="true">{s.id === activeId && <Check size={14} />}</span>
                <span className="truncate">{s.title}</span>
                <span className="cdx-session-when">{relativeTime(s.updatedAt)}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
