import { Archive, Brain, ChevronRight, FolderOpen, LayoutGrid, PanelLeft, SlidersHorizontal, SquarePen, Wrench } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { usePopover } from './use-popover.ts'
import type { SessionMeta } from '../ipc.ts'
import { FileTree } from './FileTree.tsx'
import { ProjectMapStatus } from './ProjectMapStatus.tsx' // [g6]
import { SessionList, type SessionExtras, type SessionStatus } from './SessionList.tsx'
import { SettingsRailButton } from './G7RailButton.tsx' // [g7]
import { G11RailButton } from './G11Layer.tsx' // [g11]
import { Logo } from './Logo.tsx'
import { SecurityRailButton } from './g12/SecurityRailButton.tsx' // [g12]
import { Tooltip } from './kor-ui/index.ts'
import { usePersistentState } from './use-persistent-state.ts'
import { SIDEBAR_VIEW_GROUPS, openView, viewInfo, type ViewGroup } from './view-catalog.ts'

interface RailButtonProps {
  icon: ReactNode
  label: string
  collapsed: boolean
  onClick: () => void
  disabled?: boolean
  /** Görünüm anahtarı (ör. Arşiv) için basılı durum */
  pressed?: boolean
}

function RailButton({ icon, label, collapsed, onClick, disabled, pressed }: RailButtonProps) {
  const button = (
    <button
      type="button"
      className={`rail-btn${pressed ? ' g1-pressed' : ''}`}
      onClick={onClick}
      disabled={disabled}
      aria-label={collapsed ? label : undefined}
      aria-pressed={pressed}
    >
      {icon}
      <span className="rail-btn-label">{label}</span>
    </button>
  )
  // Daraltılınca yalnız ikon kalır: etiket 400 ms gecikmeli tooltip'e taşınır (§23.1).
  return collapsed ? <Tooltip text={label}>{button}</Tooltip> : button
}

interface SidebarProps extends SessionExtras {
  collapsed: boolean
  onToggle: () => void
  root: string | null
  selected: string | null
  changed: ReadonlySet<string>
  sessions: SessionMeta[] | undefined
  sessionsError: string | null
  activeSessionId: string | null
  statusOf: (id: string) => SessionStatus
  onNewChat: () => void
  onOpenFolder: () => void
  onOpenFile: (path: string) => void
  onOpenSession: (id: string) => void
  onPinSession: (id: string, pinned: boolean) => void
  onDeleteSession: (id: string) => void
  onOpenMemory: () => void
  onOpenSystemPrompt: () => void
}

export function Sidebar(props: SidebarProps) {
  const { collapsed, root } = props
  const [tab, setTab] = useState<'chats' | 'files'>('chats')
  const [view, setView] = useState<'chats' | 'archive'>('chats')
  const rootName = root?.split(/[\\/]/).pop() ?? ''
  const archivedCount = props.sessions?.filter((s) => s.archived).length ?? 0

  const toggleArchive = () => {
    if (collapsed) props.onToggle()
    setTab('chats')
    setView((v) => (v === 'archive' && !collapsed ? 'chats' : 'archive'))
  }

  return (
    <aside className={`rail${collapsed ? ' collapsed' : ''}`} aria-label="Kenar çubuğu">
      <div className="rail-top">
        <Tooltip side="bottom" text={collapsed ? 'Kenar çubuğunu aç' : 'Kenar çubuğunu kapat'}>
          <button
            type="button"
            className="action-btn"
            onClick={props.onToggle}
            aria-label={collapsed ? 'Kenar çubuğunu aç' : 'Kenar çubuğunu kapat'}
          >
            <PanelLeft size={16} aria-hidden="true" />
          </button>
        </Tooltip>
        {/* Daraltılmış şeride logo sığmaz; yalnızca aç/kapa düğmesi kalır. */}
        {!collapsed && <Logo size={22} className="brand-logo" />}
        <span className="brand">Kor</span>
      </div>

      <div className="rail-actions">
        <RailButton icon={<SquarePen size={16} aria-hidden="true" />} label="Yeni sohbet" collapsed={collapsed} onClick={props.onNewChat} disabled={!root} />
        <RailButton
          icon={<FolderOpen size={16} aria-hidden="true" />}
          label={root ? rootName : 'Klasör aç'}
          collapsed={collapsed}
          onClick={props.onOpenFolder}
        />
      </div>

      {!collapsed && (
        <>
          <div className="segmented" role="tablist" aria-label="Kenar çubuğu içeriği">
            <span className={`segmented-pill${tab === 'files' ? ' right' : ''}`} aria-hidden="true" />
            <button
              type="button"
              role="tab"
              aria-selected={tab === 'chats'}
              onClick={() => {
                setTab('chats')
                setView('chats')
              }}
            >
              Sohbetler
            </button>
            <button type="button" role="tab" aria-selected={tab === 'files'} onClick={() => setTab('files')} disabled={!root}>Dosyalar</button>
          </div>
          <div className="rail-scroll">
            {tab === 'chats' || !root ? (
              <SessionList
                sessions={props.sessions}
                error={props.sessionsError}
                activeId={props.activeSessionId}
                statusOf={props.statusOf}
                onOpen={props.onOpenSession}
                onPin={props.onPinSession}
                onDelete={props.onDeleteSession}
                view={view}
                onExitArchive={() => setView('chats')}
                onOpenArchive={() => setView('archive')}
                sections={props.sections}
                sectionsError={props.sectionsError}
                pricing={props.pricing}
                splitSessionId={props.splitSessionId}
                onRenameSession={props.onRenameSession}
                onArchiveSession={props.onArchiveSession}
                onMoveSession={props.onMoveSession}
                onOpenSplit={props.onOpenSplit}
                onOpenWindow={props.onOpenWindow}
                onDuplicateSession={props.onDuplicateSession}
                onExportSession={props.onExportSession}
                onCreateSection={props.onCreateSection}
                onRenameSection={props.onRenameSection}
                onDeleteSection={props.onDeleteSection}
              />
            ) : (
              <FileTree rootName={rootName} selected={props.selected} changed={props.changed} onOpenFile={props.onOpenFile} />
            )}
          </div>
        </>
      )}

      <div className="rail-bottom">
        {/* Alt şerit: Ayarlar + "Araçlar" başlatıcısı (görünüm grupları + genel araçlar). Daraltılınca her grup bir ikon + açılır liste. */}
        <SettingsRailButton collapsed={collapsed} />
        <ToolsLauncher collapsed={collapsed}>
          <ProjectMapStatus root={root} collapsed={false} />
          <G11RailButton collapsed={false} />
          <SecurityRailButton sessionId={props.activeSessionId} collapsed={false} />
          <RailButton icon={<SlidersHorizontal size={16} aria-hidden="true" />} label="Geliştirici araçları" collapsed={false} onClick={() => window.dispatchEvent(new CustomEvent('g13:open', { detail: 'layers' }))} />
          <RailButton icon={<span className="g14-rail-icon" aria-hidden="true" />} label="Gözlem" collapsed={false} onClick={() => window.dispatchEvent(new Event('g14:open'))} />
          <RailButton icon={<Brain size={16} aria-hidden="true" />} label="Hafıza" collapsed={false} onClick={props.onOpenMemory} />
          <RailButton icon={<SlidersHorizontal size={16} aria-hidden="true" />} label="Sistem istemi" collapsed={false} onClick={props.onOpenSystemPrompt} />
          <RailButton icon={<Archive size={16} aria-hidden="true" />} label={archivedCount > 0 ? `Arşiv (${archivedCount})` : 'Arşiv'} collapsed={false} onClick={toggleArchive} pressed={view === 'archive' && tab === 'chats' && !collapsed} />
        </ToolsLauncher>
      </div>
    </aside>
  )
}

/** Bir grubun görünüm satırları: tıklanınca görünüm sağ panelde açılır. */
function GroupItems({ group, role }: { group: ViewGroup; role?: 'menuitem' }) {
  return (
    <>
      {group.views.map((id) => {
        const v = viewInfo(id)
        if (!v) return null
        return (
          <button key={id} type="button" role={role} className="rail-btn kor-launch-item" onClick={() => openView(id)}>
            <v.icon size={16} />
            <span className="rail-btn-label">{v.label}</span>
          </button>
        )
      })}
    </>
  )
}

const GENERAL_GROUP = { id: 'general', label: 'Genel', icon: Wrench } as const

/**
 * "Araçlar" başlatıcısı. Açık kenar çubuğunda katlanır grup başlıkları (açık/kapalı durumu bu cihazda hatırlanır);
 * daraltılmış şeritte her grup tek ikon + yana açılan liste.
 */
function ToolsLauncher({ collapsed, children }: { collapsed: boolean; children: ReactNode }) {
  const [openState, setOpenState] = usePersistentState<Record<string, boolean>>('kor.sidebar.tools', {})
  const isOpen = (id: string, fallback = false) => openState[id] ?? fallback
  const toggle = (id: string, fallback = false) => setOpenState((prev) => ({ ...prev, [id]: !(prev[id] ?? fallback) }))

  if (collapsed) {
    return (
      <>
        {SIDEBAR_VIEW_GROUPS.map((g) => (
          <RailFlyout key={g.id} icon={<g.icon size={16} aria-hidden="true" />} label={g.label}>
            <GroupItems group={g} role="menuitem" />
          </RailFlyout>
        ))}
        <RailFlyout icon={<Wrench size={16} aria-hidden="true" />} label={GENERAL_GROUP.label}>
          {children}
        </RailFlyout>
      </>
    )
  }

  const sectionOpen = isOpen('section', true)
  const groups: ReadonlyArray<{ id: string; label: string; icon: ViewGroup['icon']; body: ReactNode }> = [
    ...SIDEBAR_VIEW_GROUPS.map((g) => ({ id: g.id, label: g.label, icon: g.icon, body: <GroupItems group={g} /> })),
    { ...GENERAL_GROUP, body: children },
  ]
  return (
    <div className="kor-launcher">
      <button type="button" className="rail-btn kor-launcher-head" aria-expanded={sectionOpen} onClick={() => toggle('section', true)}>
        <LayoutGrid size={16} aria-hidden="true" />
        <span className="rail-btn-label">Araçlar</span>
        <ChevronRight size={14} className="kor-launch-chev" aria-hidden="true" />
      </button>
      {sectionOpen && (
        <div className="kor-launcher-body">
          {groups.map((g) => {
            const open = isOpen(g.id)
            return (
              <div key={g.id} className="kor-launch-group">
                <button type="button" className="kor-launch-group-head" aria-expanded={open} onClick={() => toggle(g.id)}>
                  <ChevronRight size={12} className="kor-launch-chev" aria-hidden="true" />
                  <g.icon size={14} />
                  <span className="truncate">{g.label}</span>
                </button>
                {open && (
                  <div className="kor-launch-items" role="group" aria-label={g.label}>
                    {g.body}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

/** Daraltılmış şeritte grup ikonu + sağa açılan liste. Tıklanınca kapanır (Güvenlik modal açtığından açık kalır). */
function RailFlyout({ icon, label, children }: { icon: ReactNode; label: string; children: ReactNode }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  return (
    <div className="popover-anchor kor-rail-flyout-anchor" ref={ref}>
      <Tooltip text={label}>
        <button type="button" className="rail-btn" onClick={() => setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={open} aria-label={label}>
          {icon}
          <span className="rail-btn-label">{label}</span>
        </button>
      </Tooltip>
      {open && (
        <div
          className="menu pop-in kor-rail-flyout"
          role="menu"
          aria-label={label}
          onClick={(e) => {
            const btn = (e.target as HTMLElement).closest('button')
            if (btn && !/Güvenlik/.test(btn.textContent ?? '')) setOpen(false)
          }}
        >
          <div className="menu-heading">{label}</div>
          {children}
        </div>
      )}
    </div>
  )
}
