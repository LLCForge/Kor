import { AlertTriangle, LayoutGrid, MoreHorizontal, Pin, PinOff, RotateCcw, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { confirmAction } from './ConfirmDialog.tsx'
import { usePopover } from './use-popover.ts'
import type { ExternalStatus } from '../ipc-g4.ts'
import { api, errorMessage } from './api.ts'
import { lineStats, type FileChange } from './chat-state.ts'
import { Diagnostics } from './Diagnostics.tsx'
import { DIFF_MODES, DiffView } from './DiffView.tsx'
import { FilePreview } from './FilePreview.tsx'
import { TerminalPanel } from './TerminalPanel.tsx'
import { GitPanel } from './GitPanel.tsx'
import { NotesPanel } from './NotesPanel.tsx'
import { PreviewPanel } from './PreviewPanel.tsx'
import { BrowserPanel } from './BrowserPanel.tsx'
import { LogsPanel } from './LogsPanel.tsx'
import { ChangesTree, type ChangeRowInfo } from './ChangesTree.tsx'
import { LangBadge } from './LangBadge.tsx'
import { ReviewCount } from './ReviewCount.tsx'
import { clearAllDrafts, draftPaths } from './g4-shared.ts'
import { usePersistentChoice } from './persisted.ts'
import { findPanel } from './panels/registry.ts'
import { PanelBoundary } from './PanelBoundary.tsx'
import { Tooltip } from './kor-ui/index.ts'
import { usePersistentState } from './use-persistent-state.ts'
import { BUILTIN_VIEWS, HEADER_TABS, openAllViews, viewInfo, type ViewInfo } from './view-catalog.ts'

/** Grup 4: satıra gitme, harici değişiklik uyarıları ve tanılamadan dosya açma. */
export interface G4PanelProps {
  line: number | null
  /** Aynı satıra yeniden gitme isteklerini ayırt eder */
  lineSeq: number
  external: Readonly<Record<string, ExternalStatus>>
  onOpenFile: (path: string, line?: number) => void
}

type ChangeStatus = 'new' | 'modified' | 'deleted'
const STATUS_LABEL: Record<ChangeStatus, string> = { new: 'Yeni', modified: 'Değişti', deleted: 'Silindi' }

// Değişiklik nesneleri değişmezdir; satır sayımı (diff) her olayda yeniden hesaplanmasın.
const statsCache = new WeakMap<FileChange, { added: number; removed: number }>()
function statsOf(change: FileChange): { added: number; removed: number } {
  let stats = statsCache.get(change)
  if (!stats) {
    stats = lineStats(change.before, change.after)
    statsCache.set(change, stats)
  }
  return stats
}

function statusOf(change: FileChange, external: ExternalStatus | undefined): ChangeStatus {
  if (external === 'deleted') return 'deleted'
  return change.before === null ? 'new' : 'modified'
}

interface SidePanelProps {
  changes: FileChange[]
  /** [perf] Büyük dosya içeriği getirilince store'a yazılır */
  onHydrateChange?: (path: string, before: string | null, after: string) => void
  previewPath: string | null
  onAttach: (path: string) => void
  onRevert: (path: string) => Promise<void>
  onClose: () => void
  /** Notlar/Günlükler için etkin sohbet (yoksa null) */
  sessionId?: string | null
  // [g4] opsiyonel: eski çağrılar bozulmaz
  g4?: G4PanelProps
  /** Kapanış animasyonu oynuyor (App, `--kor-dur-panel` sonra kaldırır). */
  closing?: boolean
}

type BuiltinTab = 'changes' | 'preview' | 'diagnostics' | 'terminal' | 'git' | 'notes' | 'logs' | 'devpreview' | 'browser'
/** Yerleşik sekmeler + panels/registry.ts kimlikleri */
type Tab = BuiltinTab | (string & {})
const BUILTIN_TABS: readonly string[] = BUILTIN_VIEWS.map((v) => v.id)
/** Sohbet değişince panel yeniden kurulur; seçili görünüm (ve hemen önce istenen görünüm) korunur. */
let lastTab: Tab = 'changes'
const RECENT_MAX = 6

const PANEL_WIDTH_KEY = 'cdx.panelWidth'
const DEFAULT_PANEL_WIDTH = 560
const MIN_PANEL_WIDTH = 340
function readPanelWidth(): number {
  try {
    const n = Number(window.localStorage.getItem(PANEL_WIDTH_KEY))
    return Number.isFinite(n) && n >= MIN_PANEL_WIDTH ? n : DEFAULT_PANEL_WIDTH
  } catch {
    return DEFAULT_PANEL_WIDTH
  }
}
/** Genişlik sınırı: en az MIN, en çok pencerenin %70'i (sohbet alanı sıfırlanmasın). */
function clampPanelWidth(w: number): number {
  return Math.round(Math.min(Math.max(w, MIN_PANEL_WIDTH), Math.max(MIN_PANEL_WIDTH, window.innerWidth * 0.7)))
}

/** Sol kenar sürükleme: localStorage'da hatırlanan panel genişliği. */
function usePanelResize(): { panelWidth: number; setPanelWidth: (w: number) => void; resizing: boolean; startResize: (e: React.MouseEvent) => void } {
  const [panelWidth, setWidth] = useState(readPanelWidth)
  const [resizing, setResizing] = useState(false)
  const setPanelWidth = (w: number) => {
    const next = clampPanelWidth(w)
    setWidth(next)
    try {
      window.localStorage.setItem(PANEL_WIDTH_KEY, String(next))
    } catch {
      /* tercih bu oturumla sınırlı kalır */
    }
  }
  const startResize = (e: React.MouseEvent) => {
    e.preventDefault()
    setResizing(true)
    const onMove = (ev: MouseEvent) => setWidth(clampPanelWidth(window.innerWidth - ev.clientX))
    const onUp = (ev: MouseEvent) => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
      setResizing(false)
      setPanelWidth(window.innerWidth - ev.clientX)
    }
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }
  return { panelWidth, setPanelWidth, resizing, startResize }
}

export function SidePanel({ changes, previewPath, onAttach, onRevert, onClose, sessionId = null, g4, onHydrateChange, closing = false }: SidePanelProps) {
  const { panelWidth, setPanelWidth, resizing, startResize } = usePanelResize()
  // Panel her dosya seçiminde yeniden kurulur (key), seçilen dosya varsa önizleme açılır.
  const [tab, setTabState] = useState<Tab>(previewPath ? 'preview' : lastTab)
  // Sabitlenen ve son açılan görünümler bu cihazda hatırlanır; bilinmeyen (eski) kimlikler gösterimde elenir.
  const [pinned, setPinned] = usePersistentState<string[]>('kor.views.pinned', [])
  const [recent, setRecent] = usePersistentState<string[]>('kor.views.recent', [])
  const setTab = useCallback(
    (v: Tab) => {
      setTabState(v)
      lastTab = v
      if (!HEADER_TABS.includes(v)) setRecent((prev) => [v, ...prev.filter((x) => x !== v)].slice(0, RECENT_MAX))
    },
    [setRecent],
  )
  const [selectedChange, setSelectedChange] = useState<string | null>(null)
  const [revertError, setRevertError] = useState<string | null>(null)
  const [diffMode, setDiffMode] = usePersistentChoice('g4.diffMode', DIFF_MODES, 'unified')
  // Aynı dosya yeniden açılınca (ör. tanılamadan satıra git) panel yeniden kurulmaz; önizlemeye burada geçilir.
  const openSeq = g4?.lineSeq
  const mountedSeq = useRef(openSeq)
  useEffect(() => {
    if (openSeq !== mountedSeq.current) setTab('preview')
  }, [openSeq, setTab])
  // Komut paleti / kısayol: belirli görünümü aç (App paneli zaten açmıştır).
  useEffect(() => {
    const onView = (e: Event) => {
      const v = (e as CustomEvent<string>).detail as Tab
      if (BUILTIN_TABS.includes(v) || findPanel(v)) setTab(v)
    }
    window.addEventListener('panel:view', onView)
    return () => window.removeEventListener('panel:view', onView)
  }, [setTab])
  const external = g4?.external ?? {}
  const active = changes.find((c) => c.path === selectedChange) ?? changes.at(-1)
  const currentTab: Tab = tab === 'preview' && !previewPath ? 'changes' : tab
  const activeExternal = active ? external[active.path] : undefined
  const totals = useMemo(
    () => changes.reduce((acc, c) => ({ added: acc.added + statsOf(c).added, removed: acc.removed + statsOf(c).removed }), { added: 0, removed: 0 }),
    [changes],
  )

  const revert = async (path: string) => {
    setRevertError(null)
    if (
      external[path] &&
      !(await confirmAction({
        title: 'Değişiklik geri alınsın mı?',
        message: `${path} oturum dışında da değişti. Geri alırsan dosya oturumun başındaki haline döner ve o harici değişiklikler de kaybolur.`,
        confirmLabel: 'Geri al',
        danger: true,
      }))
    ) {
      return
    }
    try {
      await onRevert(path)
    } catch (err) {
      setRevertError(errorMessage(err))
    }
  }

  const close = async () => {
    const drafts = draftPaths()
    if (
      drafts.length > 0 &&
      !(await confirmAction({
        title: 'Panel kapatılsın mı?',
        message: `${drafts.length} dosyada kaydedilmemiş değişiklik var (${drafts.join(', ')}). Panel kapanırsa bu değişiklikler atılır.`,
        confirmLabel: 'Atıp kapat',
        danger: true,
      }))
    ) {
      return
    }
    clearAllDrafts()
    onClose()
  }

  // Panel başlığı: gündelik görünümler sekme; diğerleri "Diğer" menüsünde (sabitlenen + son açılan) ve paletten.
  const available = (id: string) => (id === 'preview' ? !!previewPath : id === 'diagnostics' ? !!g4 : viewInfo(id) !== undefined)
  const registered = findPanel(currentTab)
  const currentView: ViewInfo = viewInfo(currentTab) ?? BUILTIN_VIEWS[0]!
  const CurrentIcon = currentView.icon
  const inTabs = HEADER_TABS.includes(currentTab)

  return (
    <aside className={`side-panel${closing ? ' closing' : ''}`} inert={closing} aria-label="Değişiklikler, önizleme ve tanılama" style={{ width: panelWidth }}>
      {/* [cdx] Sol kenardan sürükleyerek genişlik ayarı (cdesktop resize handle); localStorage'da hatırlanır */}
      <div
        className={`cdx-panel-resizer${resizing ? ' active' : ''}`}
        role="separator"
        aria-orientation="vertical"
        aria-label="Panel genişliği"
        title="Sürükleyerek genişlet"
        onMouseDown={startResize}
        onDoubleClick={() => setPanelWidth(DEFAULT_PANEL_WIDTH)}
      />
      <div className="panel-head">
        <div className="kor-panel-tabs" role="tablist" aria-label="Görünüm">
          {HEADER_TABS.map((id) => {
            const v = viewInfo(id)
            if (!v) return null
            return (
              <Tooltip key={id} side="bottom" text={v.label}>
                <button
                  type="button"
                  role="tab"
                  className="action-btn kor-panel-tab"
                  aria-selected={currentTab === id}
                  aria-label={v.label}
                  disabled={!available(id)}
                  onClick={() => setTab(id)}
                >
                  <v.icon size={16} />
                  {id === 'changes' && changes.length > 0 && <span className="count kor-panel-tab-count">{changes.length}</span>}
                </button>
              </Tooltip>
            )
          })}
        </div>
        {!inTabs && (
          <span className="panel-title kor-panel-current">
            <CurrentIcon size={15} aria-hidden="true" />
            <span className="truncate">{currentView.label}</span>
          </span>
        )}
        <div className="panel-head-actions">
          <PanelMoreMenu
            current={currentTab}
            pinned={pinned.filter((id) => !HEADER_TABS.includes(id) && available(id))}
            recent={recent.filter((id) => !HEADER_TABS.includes(id) && !pinned.includes(id) && available(id))}
            onSelect={setTab}
            onTogglePin={(id) => setPinned((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]))}
          />
          <button type="button" className="action-btn panel-close" onClick={() => void close()} aria-label="Paneli kapat" title="Paneli kapat">
            <X size={16} aria-hidden="true" />
          </button>
        </div>
      </div>

      <PanelBoundary resetKey={currentTab}>
      {registered ? (
        <div className="panel-body kor-panel" data-kor-panel={registered.id}>
          <registered.component sessionId={sessionId} />
        </div>
      ) : currentTab === 'terminal' ? (
        <TerminalPanel />
      ) : currentTab === 'devpreview' ? (
        <PreviewPanel />
      ) : currentTab === 'browser' ? (
        <BrowserPanel />
      ) : currentTab === 'git' ? (
        <GitPanel />
      ) : currentTab === 'notes' ? (
        <NotesPanel sessionId={sessionId} />
      ) : currentTab === 'logs' ? (
        <LogsPanel sessionId={sessionId} />
      ) : currentTab === 'diagnostics' && g4 ? (
        <Diagnostics onOpenFile={g4.onOpenFile} />
      ) : currentTab === 'preview' && previewPath ? (
        <FilePreview path={previewPath} line={g4?.line ?? null} lineSeq={g4?.lineSeq ?? 0} onAttach={onAttach} />
      ) : changes.length === 0 ? (
        <p className="muted pad">Ajan bir dosyayı değiştirdiğinde farklar burada görünecek.</p>
      ) : (
        <div className="panel-body">
          <p className="g4-change-summary">
            {changes.length} dosya · <span className="stat-add">+{totals.added}</span> <span className="stat-del">-{totals.removed}</span>
          </p>
          <ChangesTree
            rows={changes.map((c): ChangeRowInfo => {
              const ext = external[c.path]
              return { path: c.path, status: statusOf(c, ext), ...statsOf(c), ...(ext ? { external: ext } : {}) }
            })}
            selected={active?.path ?? null}
            onSelect={setSelectedChange}
          />
          {active && (
            <>
              <div className="panel-file-head">
                <LangBadge path={active.path} />
                <code title={active.path}>{active.path}</code>
                <span className="line-stats cdx-head-stats" aria-hidden="true">
                  <span className="stat-add">+{statsOf(active).added}</span>
                  <span className="stat-del">-{statsOf(active).removed}</span>
                </span>
                <ReviewCount path={active.path} />
                <div className="g4-bar-actions">
                  <div className="g4-toggle" role="group" aria-label="Diff görünümü">
                    <button type="button" aria-pressed={diffMode === 'unified'} onClick={() => setDiffMode('unified')}>
                      Birleşik
                    </button>
                    <button type="button" aria-pressed={diffMode === 'split'} onClick={() => setDiffMode('split')}>
                      Yan yana
                    </button>
                  </div>
                  <button type="button" className="btn small" onClick={() => void revert(active.path)}>
                    <RotateCcw size={14} aria-hidden="true" /> Geri al
                  </button>
                </div>
              </div>
              {activeExternal && (
                <p className="g4-banner" role="alert">
                  <AlertTriangle size={14} aria-hidden="true" />
                  <span>
                    {activeExternal === 'deleted'
                      ? 'Bu dosya oturum dışında silindi. Aşağıdaki fark ajanın son yazdığı hali gösterir.'
                      : 'Bu dosya oturum dışında (başka bir editör veya komutla) değişti. Aşağıdaki fark ajanın son yazdığı hali gösterir.'}
                  </span>
                </p>
              )}
              {revertError && <p className="error-text pad" role="alert">{revertError}</p>}
              {active.lazy ? (
                <LazyChange change={active} sessionId={sessionId} onLoaded={onHydrateChange} />
              ) : (
                <DiffView change={active} mode={diffMode} />
              )}
            </>
          )}
        </div>
      )}
      </PanelBoundary>
    </aside>
  )
}

/** Panel başlığındaki "Diğer görünümler" menüsü: yalnız sabitlenen ve son açılan görünümler + "Tüm görünümler…" (palet). */
function PanelMoreMenu({
  current,
  pinned,
  recent,
  onSelect,
  onTogglePin,
}: {
  current: Tab
  pinned: readonly string[]
  recent: readonly string[]
  onSelect: (id: Tab) => void
  onTogglePin: (id: string) => void
}) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const row = (id: string, isPinned: boolean) => {
    const v = viewInfo(id)
    if (!v) return null
    const pinLabel = isPinned ? `${v.label}: sabitlemeyi kaldır` : `${v.label}: sabitle`
    return (
      <div key={id} className="kor-view-row">
        <button
          type="button"
          role="menuitemradio"
          aria-checked={id === current}
          className="menu-row"
          onClick={() => {
            onSelect(id)
            setOpen(false)
          }}
        >
          <v.icon size={16} />
          <span className="panel-view-label truncate">{v.label}</span>
        </button>
        <button type="button" role="menuitem" className="action-btn kor-view-pin" aria-pressed={isPinned} aria-label={pinLabel} title={pinLabel} onClick={() => onTogglePin(id)}>
          {isPinned ? <PinOff size={14} aria-hidden="true" /> : <Pin size={14} aria-hidden="true" />}
        </button>
      </div>
    )
  }
  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip side="bottom" text="Diğer görünümler">
        <button
          type="button"
          className="action-btn panel-view-trigger"
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label="Diğer görünümler"
        >
          <MoreHorizontal size={16} aria-hidden="true" />
        </button>
      </Tooltip>
      {open && (
        <div className="menu menu-left pop-in panel-view-menu" role="menu" aria-label="Diğer görünümler">
          {pinned.length > 0 && <div className="menu-heading">Sabitlenenler</div>}
          {pinned.map((id) => row(id, true))}
          {recent.length > 0 && <div className="menu-heading">Son açılanlar</div>}
          {recent.map((id) => row(id, false))}
          {pinned.length === 0 && recent.length === 0 && <p className="menu-note">Açtığın görünümler burada listelenir; sık kullandıklarını sabitleyebilirsin.</p>}
          <div className="g1-menu-sep" role="separator" />
          <button
            type="button"
            role="menuitem"
            className="menu-row"
            onClick={() => {
              setOpen(false)
              openAllViews()
            }}
          >
            <LayoutGrid size={16} aria-hidden="true" />
            <span className="panel-view-label">Tüm görünümler…</span>
            <kbd className="cdx-menu-hint">G V</kbd>
          </button>
        </div>
      )}
    </div>
  )
}

/** [perf] Büyük dosya: içerik IPC'den akmadı; istenince ana süreçten alınır ve store'a yazılır. */
function LazyChange({ change, sessionId, onLoaded }: { change: FileChange; sessionId: string | null; onLoaded?: (path: string, before: string | null, after: string) => void }) {
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const kb = Math.round((change.lazy?.bytes ?? 0) / 1024)
  const load = () => {
    if (!sessionId) return
    setBusy(true)
    setErr(null)
    void api.getFileChange(sessionId, change.path)
      .then((r) => {
        if (!r) throw new Error('İçerik alınamadı (sohbet kapalı olabilir).')
        onLoaded?.(change.path, r.before, r.after)
      })
      .catch((e: unknown) => setErr(errorMessage(e)))
      .finally(() => setBusy(false))
  }
  return (
    <div className="cdx-lazy-change">
      <p className="muted">Büyük dosya ({kb} KB): fark ağı yormamak için hemen yüklenmedi.</p>
      <button type="button" className="btn small" onClick={load} disabled={busy || !sessionId}>{busy ? 'Yükleniyor…' : 'Farkı yükle'}</button>
      {err && <p className="error-text">{err}</p>}
    </div>
  )
}
