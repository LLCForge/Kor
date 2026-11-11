import {
  AppWindow,
  Archive,
  ArchiveRestore,
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  Archive as ArchiveIcon,
  CircleAlert,
  Columns2,
  Copy,
  Folder,
  GitFork,
  FileDown,
  Layers,
  FileText,
  FolderInput,
  FolderPlus,
  FolderX,
  GitBranch,
  History,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Play,
  Search,
  Trash2,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { confirmAction } from './ConfirmDialog.tsx'
import { useAnchoredMenu } from './use-anchored-menu.ts'
import type { Section } from '../../core/sections.ts'
import type { SessionMeta } from '../ipc.ts'
import { errorMessage } from './api.ts'
import { SessionTreeBadges } from './prefs/SessionBadges.tsx' // [ADR 0050] §28.6 etiket ve renk
import { filterSessions, formatUsage, groupSessions, projectName, relativeTime, type ModelPricing } from './session-meta.ts'
import { usePopover } from './use-popover.ts'
import { openView } from './view-catalog.ts'

export type SessionStatus = 'idle' | 'running' | 'waiting' | 'unread'

const STATUS_LABEL: Record<SessionStatus, string> = {
  idle: '',
  running: 'Çalışıyor',
  waiting: 'Onay bekliyor',
  unread: 'Yeni yanıt',
}

const MAX_SESSION_TITLE = 120
const MAX_SECTION_NAME = 60

/** Sohbet ve bölüm yönetimi: yeniden adlandırma, arşiv, bölümler, yan yana/yeni pencere, kullanım özeti. */
export interface SessionExtras {
  sections: Section[] | undefined
  sectionsError: string | null
  pricing: ModelPricing | undefined
  /** Yan bölmede açık sohbet */
  splitSessionId: string | null
  /** title null: kullanıcı adı kaldırılır, otomatik başlığa dönülür */
  onRenameSession: (id: string, title: string | null) => void
  onArchiveSession: (id: string, archived: boolean) => void
  onMoveSession: (id: string, sectionId: string | null) => void
  onOpenSplit: (id: string) => void
  onOpenWindow: (id: string) => void
  /** [cdx] Çoğalt (spinOff=false) / Dallandır (spinOff=true) */
  onDuplicateSession: (id: string, spinOff: boolean) => void
  /** [cdx] Markdown dışa aktarma (kaydetme penceresi) */
  onExportSession: (id: string) => void
  /** Hata satır içinde gösterilsin diye reddedilen söz döner */
  onCreateSection: (name: string) => Promise<void>
  onRenameSection: (id: string, name: string) => void
  onDeleteSection: (id: string) => void
}

/** cdesktop satır durumu: ▶ (turuncu) çalışıyor, ✋/! onay bekliyor, ● okunmamış; boşta ikon yok (meta satırında gösterilir). */
function StatusMark({ status }: { status: SessionStatus }) {
  if (status === 'idle') return null
  if (status === 'running') return <Play size={12} className="session-status running" fill="currentColor" role="img" aria-label={STATUS_LABEL.running} />
  if (status === 'waiting') return <CircleAlert size={13} className="session-status waiting" role="img" aria-label={STATUS_LABEL.waiting} />
  return <span className="session-status unread" role="img" aria-label={STATUS_LABEL.unread} title={STATUS_LABEL.unread} />
}

/** İkinci satır (cdesktop): durum ikonu + "8dk önce" solda, "📄 N +X −Y" sağda. */
function SessionMetaLine({ session, status }: { session: SessionMeta; status: SessionStatus }) {
  const stats = session.stats
  return (
    <span className="cdx-session-meta">
      <StatusMark status={status} />
      <span className="cdx-session-time">{relativeTime(session.updatedAt)}</span>
      {stats && stats.files > 0 && (
        <span className="cdx-session-stats" title={`${stats.files} dosya değişti`}>
          <FileText size={11} aria-hidden="true" />
          {stats.files}
          <span className="stat-add">+{stats.added}</span>
          <span className="stat-del">-{stats.removed}</span>
        </span>
      )}
    </span>
  )
}

interface InlineInputProps {
  initial: string
  label: string
  maxLength: number
  /** keyboard: Enter ile bitirildi (odak satıra geri döner); fare ile dışarı tıklanınca false */
  onSubmit: (value: string, keyboard: boolean) => void
  onCancel: () => void
}

/** Enter kaydeder, Esc vazgeçer, dışarı tıklamak kaydeder. */
function InlineInput({ initial, label, maxLength, onSubmit, onCancel }: InlineInputProps) {
  const done = useRef(false)
  const finish = (value: string | null, keyboard: boolean) => {
    if (done.current) return
    done.current = true
    if (value === null) onCancel()
    else onSubmit(value, keyboard)
  }
  return (
    <input
      className="g1-inline-input"
      autoFocus
      defaultValue={initial}
      aria-label={label}
      maxLength={maxLength}
      onFocus={(e) => e.currentTarget.select()}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          finish(e.currentTarget.value, true)
        } else if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          finish(null, true)
        }
      }}
      onBlur={(e) => finish(e.currentTarget.value, false)}
    />
  )
}

function MenuRow({ icon, children, onClick, danger, disabled, hint }: { icon: ReactNode; children: ReactNode; onClick: () => void; danger?: boolean; disabled?: boolean; hint?: string }) {
  return (
    <button type="button" role="menuitem" className={`menu-row${danger ? ' danger' : ''}`} onClick={onClick} disabled={disabled}>
      {icon}
      <span className="truncate">{children}</span>
      {hint && <kbd className="cdx-menu-hint">{hint}</kbd>}
    </button>
  )
}

interface RowContext {
  activeId: string | null
  statusOf: (id: string) => SessionStatus
  onOpen: (id: string) => void
  onPin: (id: string, pinned: boolean) => void
  onDelete: (id: string) => void
  extras: SessionExtras
}

function SessionRow({ session, index, ctx }: { session: SessionMeta; index: number; ctx: RowContext }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const [editing, setEditing] = useState(false)
  const [moveOpen, setMoveOpen] = useState(false)
  const openButton = useRef<HTMLButtonElement>(null)
  const moreRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const menuStyle = useAnchoredMenu(open, moreRef, menuRef)
  const { extras } = ctx
  const active = session.id === ctx.activeId
  // [cdx] W R akoru: etkin sohbeti satır içi yeniden adlandır
  useEffect(() => {
    if (!active) return
    const onRename = (e: Event) => { if ((e as CustomEvent<string>).detail === session.id) setEditing(true) }
    window.addEventListener('cdx:rename', onRename)
    return () => window.removeEventListener('cdx:rename', onRename)
  }, [active, session.id])
  const inSplit = session.id === extras.splitSessionId
  const usage = formatUsage(session.usage, extras.pricing)
  const sections = extras.sections ?? []

  const closeMenu = () => {
    setOpen(false)
    setMoveOpen(false)
  }
  const act = (fn: () => void) => () => {
    closeMenu()
    fn()
  }
  const endEdit = (keyboard: boolean) => {
    setEditing(false)
    if (keyboard) requestAnimationFrame(() => openButton.current?.focus())
  }

  return (
    <li className="session-item" style={{ '--i': Math.min(index, 12) } as CSSProperties}>
      <div className={`session-row${active ? ' active' : ''}${inSplit ? ' g1-in-split' : ''}${open ? ' menu-open' : ''}`} ref={ref}>
        {editing ? (
          <InlineInput
            initial={session.title}
            label="Sohbet adı"
            maxLength={MAX_SESSION_TITLE}
            onSubmit={(value, keyboard) => {
              endEdit(keyboard)
              const title = value.trim()
              if (title !== session.title) extras.onRenameSession(session.id, title || null)
            }}
            onCancel={() => endEdit(true)}
          />
        ) : (
          <button
            ref={openButton}
            type="button"
            className="session-open g1-two-line cdx-two-line"
            onClick={() => ctx.onOpen(session.id)}
            onDoubleClick={() => setEditing(true)}
            onKeyDown={(e) => {
              if (e.key === 'F2') {
                e.preventDefault()
                setEditing(true)
              }
            }}
            title={usage ? `${session.title}\n${usage}` : session.title}
            aria-current={active ? 'true' : undefined}
          >
            <span className="g1-session-text">
              <span className="session-title">
                <SessionTreeBadges session={session} />
                {session.title}
              </span>
              <SessionMetaLine session={session} status={ctx.statusOf(session.id)} />
            </span>
          </button>
        )}
        {!editing && (
          <button
            type="button"
            ref={moreRef}
            className="session-more"
            onClick={() => (open ? closeMenu() : setOpen(true))}
            aria-label={`${session.title} seçenekleri`}
            aria-haspopup="menu"
            aria-expanded={open}
          >
            <MoreHorizontal size={16} aria-hidden="true" />
          </button>
        )}
        {open && (
          <div className="menu session-menu g1-session-menu pop-in" role="menu" ref={menuRef} style={menuStyle}>
            <MenuRow icon={<Pencil size={14} aria-hidden="true" />} onClick={act(() => setEditing(true))} hint="W R">
              Yeniden adlandır
            </MenuRow>
            <MenuRow icon={<Copy size={14} aria-hidden="true" />} onClick={act(() => extras.onDuplicateSession(session.id, false))} hint="W D">
              Çoğalt
            </MenuRow>
            <MenuRow icon={<GitFork size={14} aria-hidden="true" />} onClick={act(() => extras.onDuplicateSession(session.id, true))}>
              Dallandır (spin off)
            </MenuRow>
            <MenuRow icon={<History size={14} aria-hidden="true" />} onClick={act(() => openView('replay', session.id))}>
              Oturum kayıtları
            </MenuRow>
            <MenuRow icon={<GitBranch size={14} aria-hidden="true" />} onClick={act(() => openView('forks', session.id))}>
              Konuşma dalları
            </MenuRow>
            <MenuRow icon={<FileDown size={14} aria-hidden="true" />} onClick={act(() => extras.onExportSession(session.id))} hint="W E">
              Markdown olarak dışa aktar
            </MenuRow>
            <MenuRow
              icon={session.pinned ? <PinOff size={14} aria-hidden="true" /> : <Pin size={14} aria-hidden="true" />}
              onClick={act(() => ctx.onPin(session.id, !session.pinned))}
              hint="W P"
            >
              {session.pinned ? 'Sabitlemeyi kaldır' : 'Sabitle'}
            </MenuRow>
            <button type="button" role="menuitem" className="menu-row" aria-expanded={moveOpen} onClick={() => setMoveOpen((o) => !o)}>
              <FolderInput size={14} aria-hidden="true" />
              <span className="truncate">Bölüme taşı</span>
              <ChevronRight size={14} className={`g1-sub-chevron${moveOpen ? ' open' : ''}`} aria-hidden="true" />
            </button>
            {moveOpen && (
              <div className="g1-submenu" role="group" aria-label="Bölümler">
                {sections.length === 0 && <p className="menu-note">Önce kenar çubuğunda “Yeni bölüm” oluştur.</p>}
                {sections.map((section) => (
                  <button
                    key={section.id}
                    type="button"
                    role="menuitemradio"
                    aria-checked={session.sectionId === section.id}
                    className="menu-row"
                    onClick={act(() => extras.onMoveSession(session.id, section.id))}
                  >
                    <span className="g1-check" aria-hidden="true">{session.sectionId === section.id && <Check size={14} />}</span>
                    <span className="truncate">{section.name}</span>
                  </button>
                ))}
                {session.sectionId && (
                  <MenuRow icon={<FolderX size={14} aria-hidden="true" />} onClick={act(() => extras.onMoveSession(session.id, null))}>
                    Bölümden çıkar
                  </MenuRow>
                )}
              </div>
            )}
            <MenuRow icon={<Columns2 size={14} aria-hidden="true" />} onClick={act(() => extras.onOpenSplit(session.id))} disabled={active || inSplit}>
              Yan yana aç
            </MenuRow>
            <MenuRow icon={<AppWindow size={14} aria-hidden="true" />} onClick={act(() => extras.onOpenWindow(session.id))}>
              Yeni pencerede aç
            </MenuRow>
            <MenuRow
              icon={session.archived ? <ArchiveRestore size={14} aria-hidden="true" /> : <Archive size={14} aria-hidden="true" />}
              onClick={act(() => extras.onArchiveSession(session.id, !session.archived))}
              hint="W A"
            >
              {session.archived ? 'Arşivden çıkar' : 'Arşivle'}
            </MenuRow>
            {usage && (
              <p className="g1-menu-usage" role="note" aria-label={`Kullanım: ${usage}`}>
                {usage}
              </p>
            )}
            <div className="g1-menu-sep" role="separator" />
            <MenuRow
              icon={<Trash2 size={14} aria-hidden="true" />}
              danger
              onClick={act(() => {
                void confirmAction({
                  title: 'Sohbet silinsin mi?',
                  message: `"${session.title}" kalıcı olarak silinecek. Bu işlem geri alınamaz; saklamak istersen Arşivle'yi kullan.`,
                  confirmLabel: 'Sil',
                  danger: true,
                }).then((ok) => {
                  if (ok) ctx.onDelete(session.id)
                })
              })}
              hint="W X"
            >
              Sil
            </MenuRow>
          </div>
        )}
      </div>
    </li>
  )
}

interface GroupProps {
  title: string
  sessions: SessionMeta[]
  ctx: RowContext
  /** Başlık satırının sağındaki işlemler (bölüm menüsü) */
  actions?: ReactNode
  /** Başlık yerine gösterilecek düzenleme alanı */
  editor?: ReactNode
  emptyText?: string
}

function Group({ title, sessions, ctx, actions, editor, emptyText }: GroupProps) {
  const [collapsed, setCollapsed] = useState(false)
  const head = editor ?? (
    <button type="button" className="group-head" onClick={() => setCollapsed((c) => !c)} aria-expanded={!collapsed}>
      <span className="truncate">{title}</span>
      <ChevronDown size={12} className={`group-chevron${collapsed ? ' collapsed' : ''}`} aria-hidden="true" />
    </button>
  )
  return (
    <section className="session-group" aria-label={title}>
      {actions || editor ? (
        <div className="g1-group-head-row">
          {head}
          {!editor && actions}
        </div>
      ) : (
        head
      )}
      <div className={`collapse${collapsed ? '' : ' open'}`}>
        <div className="collapse-inner">
          {sessions.length === 0 && emptyText ? (
            <p className="tree-status g1-empty-section">{emptyText}</p>
          ) : (
            <ul className="session-list">
              {sessions.map((s, i) => (
                <SessionRow key={s.id} session={s} index={i} ctx={ctx} />
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  )
}

function SectionGroup({ section, sessions, ctx }: { section: Section; sessions: SessionMeta[]; ctx: RowContext }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const [renaming, setRenaming] = useState(false)
  const moreRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const menuStyle = useAnchoredMenu(open, moreRef, menuRef)
  const { extras } = ctx
  const actions = (
    <div className="g1-section-actions" ref={ref}>
      <button
        type="button"
        ref={moreRef}
        className="session-more g1-section-more"
        onClick={() => setOpen((o) => !o)}
        aria-label={`${section.name} bölümü seçenekleri`}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <MoreHorizontal size={14} aria-hidden="true" />
      </button>
      {open && (
        <div className="menu session-menu pop-in" role="menu" ref={menuRef} style={menuStyle}>
          <MenuRow
            icon={<Pencil size={14} aria-hidden="true" />}
            onClick={() => {
              setOpen(false)
              setRenaming(true)
            }}
          >
            Yeniden adlandır
          </MenuRow>
          <MenuRow
            icon={<Trash2 size={14} aria-hidden="true" />}
            danger
            onClick={() => {
              setOpen(false)
              void confirmAction({
                title: 'Bölüm silinsin mi?',
                message: `"${section.name}" bölümü kaldırılacak. İçindeki sohbetler silinmez, proje gruplarına döner.`,
                confirmLabel: 'Bölümü sil',
                danger: true,
              }).then((ok) => {
                if (ok) extras.onDeleteSection(section.id)
              })
            }}
          >
            Bölümü sil
          </MenuRow>
        </div>
      )}
    </div>
  )
  const editor = renaming ? (
    <InlineInput
      initial={section.name}
      label="Bölüm adı"
      maxLength={MAX_SECTION_NAME}
      onSubmit={(value) => {
        setRenaming(false)
        const name = value.trim()
        if (name && name !== section.name) extras.onRenameSection(section.id, name)
      }}
      onCancel={() => setRenaming(false)}
    />
  ) : undefined
  return <Group title={section.name} sessions={sessions} ctx={ctx} actions={actions} editor={editor} emptyText="Boş bölüm. Sohbetleri ⋯ menüsünden taşı." />
}

function NewSection({ onCreate }: { onCreate: (name: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const create = (value: string) => {
    setEditing(false)
    const name = value.trim()
    if (!name) return
    setError(null)
    onCreate(name).catch((err: unknown) => setError(errorMessage(err)))
  }
  return (
    <div className="g1-new-section">
      {editing ? (
        <InlineInput initial="" label="Yeni bölüm adı" maxLength={MAX_SECTION_NAME} onSubmit={create} onCancel={() => setEditing(false)} />
      ) : (
        <button type="button" className="group-head" onClick={() => setEditing(true)}>
          <FolderPlus size={12} aria-hidden="true" />
          <span>Yeni bölüm</span>
        </button>
      )}
      {error && (
        <p className="tree-status error" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}

interface SessionListProps extends SessionExtras {
  sessions: SessionMeta[] | undefined
  error: string | null
  activeId: string | null
  statusOf: (id: string) => SessionStatus
  onOpen: (id: string) => void
  onPin: (id: string, pinned: boolean) => void
  onDelete: (id: string) => void
  view: 'chats' | 'archive'
  onExitArchive: () => void
  /** [cdx] Alt satır "Arşivi gör N" */
  onOpenArchive?: () => void
}

/** Claude'daki gibi: "Sabitlenmiş", kullanıcı bölümleri, sonra proje klasörüne göre gruplar (en son kullanılan üstte). Arşiv ayrı görünümde. */
export function SessionList({ sessions, error, activeId, statusOf, onOpen, onPin, onDelete, view, onExitArchive, onOpenArchive, ...extras }: SessionListProps) {
  const [query, setQuery] = useState('')
  // [cdx] cdesktop filtre pill'leri: Proje ⌄ ve Durum ⌄; katman ikonu: duruma göre grupla (Dikkat gerekiyor / Çalışıyor / Boşta)
  const [projectFilter, setProjectFilter] = useState<string>('')
  const [statusFilter, setStatusFilter] = useState<'all' | 'running' | 'waiting' | 'unread'>('all')
  const [byStatus, setByStatus] = useState(false)
  const projects = useMemo(() => [...new Set((sessions ?? []).filter((s) => !s.archived).map((s) => s.root))], [sessions])
  const filtered = useMemo(
    () => filterSessions(sessions ?? [], query).filter((s) => (!projectFilter || s.root === projectFilter) && (statusFilter === 'all' || statusOf(s.id) === statusFilter)),
    [sessions, query, projectFilter, statusFilter, statusOf],
  )
  const groups = useMemo(() => groupSessions(filtered, extras.sections ?? []), [filtered, extras.sections])

  if (error) return <p className="tree-status error">{error}</p>
  if (!sessions) return <p className="tree-status">Yükleniyor...</p>

  const ctx: RowContext = { activeId, statusOf, onOpen, onPin, onDelete, extras }

  if (view === 'archive') {
    return (
      <nav className="sessions" aria-label="Arşivlenmiş sohbetler">
        <div className="g1-archive-head">
          <button type="button" className="action-btn" onClick={onExitArchive} aria-label="Sohbetlere dön" title="Sohbetlere dön">
            <ArrowLeft size={14} aria-hidden="true" />
          </button>
          <span>Arşiv</span>
        </div>
        {groups.archived.length === 0 ? (
          <p className="tree-status">Arşivlenmiş sohbet yok.</p>
        ) : (
          <ul className="session-list">
            {groups.archived.map((s, i) => (
              <SessionRow key={s.id} session={s} index={i} ctx={ctx} />
            ))}
          </ul>
        )}
      </nav>
    )
  }

  const visible = groups.pinned.length + groups.projects.length + groups.sections.reduce((n, g) => n + g.sessions.length, 0)
  return (
    <nav className="sessions" aria-label="Geçmiş sohbetler">
      {/* cdesktop: Workspaces listesinin üstünde arama kutusu */}
      <label className="cdx-search">
        <Search size={13} aria-hidden="true" />
        <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Ara…" aria-label="Sohbetlerde ara" spellCheck={false} />
      </label>
      {/* Süzgeçler ancak süzülecek sohbet varken görünür; boş listede yalnızca arama ve ipucu kalır. */}
      {sessions.length > 0 && <div className="cdx-filters">
        <label className="cdx-pill" title="Projeye göre süz">
          <Folder size={12} aria-hidden="true" />
          <select value={projectFilter} onChange={(e) => setProjectFilter(e.target.value)} aria-label="Proje filtresi">
            <option value="">Proje: Tümü</option>
            {projects.map((r) => <option key={r} value={r}>{projectName(r)}</option>)}
          </select>
        </label>
        <label className="cdx-pill" title="Duruma göre süz">
          <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value as typeof statusFilter)} aria-label="Durum filtresi">
            <option value="all">Durum: Tümü</option>
            <option value="running">Çalışıyor</option>
            <option value="waiting">Onay bekliyor</option>
            <option value="unread">Okunmamış</option>
          </select>
        </label>
        <button type="button" className={`action-btn cdx-group-toggle${byStatus ? ' active' : ''}`} onClick={() => setByStatus((b) => !b)} aria-pressed={byStatus} aria-label="Duruma göre grupla" title={byStatus ? 'Projeye göre grupla' : 'Duruma göre grupla'}>
          <Layers size={13} aria-hidden="true" />
        </button>
      </div>}
      <NewSection onCreate={extras.onCreateSection} />
      {extras.sectionsError && <p className="tree-status error">{extras.sectionsError}</p>}
      {sessions.length === 0 && <p className="tree-status">Henüz kayıtlı sohbet yok. İlk mesajından sonra burada görünecek.</p>}
      {sessions.length > 0 && query && filtered.length === 0 && <p className="tree-status">“{query}” ile eşleşen sohbet yok.</p>}
      {sessions.length > 0 && !query && visible === 0 && <p className="tree-status">Tüm sohbetler arşivde.</p>}
      {byStatus ? (
        (() => {
          const live = filtered.filter((s) => !s.archived)
          const attention = live.filter((s) => { const st = statusOf(s.id); return st === 'waiting' || st === 'unread' })
          const running = live.filter((s) => statusOf(s.id) === 'running')
          const idle = live.filter((s) => statusOf(s.id) === 'idle')
          return (
            <>
              <Group title={`Dikkat gerekiyor · ${attention.length}`} sessions={attention} ctx={ctx} emptyText="Onay bekleyen ya da okunmamış sohbet yok." />
              <Group title={`Çalışıyor · ${running.length}`} sessions={running} ctx={ctx} emptyText="Çalışan sohbet yok." />
              <Group title={`Boşta · ${idle.length}`} sessions={idle} ctx={ctx} emptyText="Boşta sohbet yok." />
            </>
          )
        })()
      ) : (
        <>
          {groups.pinned.length > 0 && <Group title="Sabitlenmiş" sessions={groups.pinned} ctx={ctx} />}
          {groups.sections.map(({ section, sessions: list }) => (
            <SectionGroup key={section.id} section={section} sessions={list} ctx={ctx} />
          ))}
          {groups.projects.map(([name, list]) => (
            <Group key={name} title={name} sessions={list} ctx={ctx} />
          ))}
        </>
      )}
      {onOpenArchive && (
        <button type="button" className="cdx-archive-foot" onClick={onOpenArchive}>
          <ArchiveIcon size={13} aria-hidden="true" /> <span>Arşivi gör</span> <span className="cdx-archive-count">{groups.archived.length}</span>
        </button>
      )}
    </nav>
  )
}
