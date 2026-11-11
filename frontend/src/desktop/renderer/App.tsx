import { PrefsLayer } from './prefs/PrefsLayer.tsx' // [ADR 0050]
import { HelpCircle, Keyboard, PanelRight, Settings as SettingsIcon } from 'lucide-react'
import { FloatingRail } from './FloatingRail.tsx'
import { ComposerStatusRow } from './ComposerStatusRow.tsx'
import { CommandPalette, ShortcutsDialog, type PaletteActions } from './CommandPalette.tsx'
import { openG7Dialog } from './g7-bus.ts'
import { clearReviewComments, getReviewComments, mergeReviewIntoMessage } from './review-comments.ts'
import { clearPreviewSelections, getPreviewSelections, mergePreviewIntoMessage } from './preview-store.ts'
import { OPEN_QUEUE_EVENT } from './shellui/TitleItems.tsx'
import { useActivityReporter, useNotificationSound, useShellBridge, type ShellHandlers } from './shellui/bridge.ts'
import { openG16Panel } from './g16-bus.ts'
import { latestTodos } from './g3-state.ts'
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { Approval } from '../../core/agent.ts'
import type { FileDecisions } from '../ipc-review.ts' // [review]
import { approveWithFiles } from './review/review-api.ts' // [review]
import type { PermissionMode } from '../../core/permissions.ts'
import { api, errorMessage } from './api.ts'
import { initialChatState, isLiveItem, lineStats, liveActions, needsTranscript, storeReducer, type ChatAction } from './chat-state.ts'
import { createStreamCoalescer } from './stream-coalescer.ts'
import { Composer } from './Composer.tsx'
import type { SessionStatus } from './SessionList.tsx'
import { SidePanel } from './SidePanel.tsx'
import { Sidebar } from './Sidebar.tsx'
import { TitleBar } from './TitleBar.tsx'
import { MemoryDialog, SystemPromptDialog } from './SystemPromptDialog.tsx'
import { Thread } from './Thread.tsx'
import { HomeEmpty } from './HomeEmpty.tsx'
import { webviewApi } from '../ipc-webview.ts'

// [g1] importlar
import { SplitPane } from './SplitView.tsx'
import { useSessionExtras } from './use-session-extras.ts'

// [g2] importlar
import { useG2App } from './g2/use-g2-app.ts'

// [g3] importlar
import { g3Actions } from './g3-bridge.ts'

// [g4] importlar
import { useG4 } from './use-g4.tsx'
import { ConfirmHost, confirmAction } from './ConfirmDialog.tsx'
import { SystemOverlay } from './system/SystemOverlay.tsx' // §19 sağlık kartları + denetim göstergesi (ADR 0059)

// [g5] importlar

// [g6] importlar
import { ProjectMapPrompt } from './ProjectMapStatus.tsx'

// [g9] importlar

// [g10] importlar
import { G10Layer } from './G10Layer.tsx'
import { G30MemoryLayer } from './G30MemoryLayer.tsx'

// [g11] importlar
import { G11Layer } from './G11Layer.tsx'

// [g12] importlar
import { BulkDeleteHost } from './g12/BulkDeleteHost.tsx'
import { OsintHost } from './osint/useOsint.ts' // [osint] ADR 0052
import { BatchEditDialog } from './osint/BatchEditDialog.tsx' // [osint] §27.6
import { osextraApi } from './osint/osextra-bridge.ts' // [osint] §27.10 Handoff

// [g13] importlar
import { G13Layer } from './g13/G13Layer.tsx'

// [g14] importlar
import { G14Layer } from './G14Layer.tsx'

// [g15] importlar

// [g16] importlar
import { G16Panel } from './G16Panel.tsx'
import { useG16Memory } from './use-g16-memory.ts'

// [g18] importlar
import { G18PasteChart } from './G18PasteChart.tsx'

// [g19] importlar

// [g20] importlar
import { G20Layer } from './G20Layer.tsx'

// [g21] importlar
import { useG21DraftInsert } from './g21/quote.ts'
import { KorChrome } from './KorChrome.tsx'
import { usePresence } from './use-presence.ts'

// [g22] importlar

// [g7] importlar
import { G7Layer } from './G7Layer.tsx'
import { Tooltip } from './kor-ui/index.ts'
import { displayPath } from './display-path.ts'

/** Kenar çubuğunun kendiliğinden daraldığı pencere genişliği (CSS px; styles.css çekmece kırılımıyla aynı). */
const NARROW_BELOW = 900
const NARROW_QUERY = `(max-width: ${NARROW_BELOW}px)`

const isFsKey = (key: unknown) => Array.isArray(key) && (key[0] === 'dir' || key[0] === 'file')

export function App() {
  const { data: appState, error: stateError, mutate: refreshState } = useSWR('state', () => api.getState())
  const { data: models, error: modelsError } = useSWR(appState?.hasApiKey ? 'models' : null, () => api.listModels(), {
    revalidateOnFocus: false,
  })
  // Listed with or without a provider key (the first run asks for nothing).
  const { data: sessions, error: sessionsError, mutate: refreshSessions } = useSWR(appState ? 'sessions' : null, () => api.listSessions())
  useEffect(() => api.onSessionsChanged(() => void refreshSessions()), [refreshSessions])
  const { mutate } = useSWRConfig()

  // Her sohbetin durumu kimliğine göre tutulur: arka planda çalışan sohbetler de olayları almaya devam eder.
  const [store, dispatchStore] = useReducer(storeReducer, {})
  // Yeni sohbet taslağı için kimliği arayüz üretir; ilk mesajla ana süreçte oturum açılır.
  const [activeId, setActiveId] = useState<string>(() => crypto.randomUUID())
  const [unread, setUnread] = useState<ReadonlySet<string>>(new Set())
  const [draft, setDraft] = useState('')
  // [cdx] Paneller (Git çakışma şeridi vb.) taslağa yazmak için pencere olayı kullanır
  useEffect(() => {
    const onDraft = (e: Event) => setDraft(String((e as CustomEvent<string>).detail ?? ''))
    window.addEventListener('cdx:draft', onDraft)
    return () => window.removeEventListener('cdx:draft', onDraft)
  }, [])
  // §27.3 dal açılışı (ForkHere, dallar paneli): dalın çalışma kopyası varsa önce o klasöre geçilir.
  const forkOpenRef = useRef<((id: string) => Promise<void>) | null>(null)
  const forkReportRef = useRef<(err: unknown) => void>(() => undefined)
  useEffect(() => {
    const onForkOpen = (e: Event) => {
      const d = (e as CustomEvent<{ id: string; worktree: string | null }>).detail
      const open = forkOpenRef.current
      if (!d?.id || !open) return
      void (d.worktree ? api.setWorkspacePath(d.worktree).then(() => open(d.id)) : open(d.id)).catch((err: unknown) => forkReportRef.current(err))
    }
    window.addEventListener('kor:fork-open', onForkOpen)
    return () => window.removeEventListener('kor:fork-open', onForkOpen)
  }, [])
  // `/<komut>` (parity-slash.ts) ve başka yerler bir paneli açmak için: kapalıysa sağ paneli de açar.
  const openPanelRef = useRef<(view: string) => void>(() => undefined)
  useEffect(() => {
    const onPanelOpen = (e: Event) => {
      const view = (e as CustomEvent<string>).detail
      if (typeof view === 'string' && view) openPanelRef.current(view)
    }
    window.addEventListener('kor:panel-open', onPanelOpen)
    return () => window.removeEventListener('kor:panel-open', onPanelOpen)
  }, [])
  // §21.1: drops outside the composer (hooks stay above the early returns below).
  const dropOutsideRef = useRef<(paths: string[]) => void>(() => undefined)
  useEffect(() => {
    const on = (e: Event) => {
      const paths = (e as CustomEvent<string[]>).detail
      if (Array.isArray(paths) && paths.length > 0) dropOutsideRef.current(paths)
    }
    window.addEventListener('kor:drop-outside', on)
    return () => window.removeEventListener('kor:drop-outside', on)
  }, [])
  const [attachments, setAttachments] = useState<string[]>([])
  const [previewPath, setPreviewPath] = useState<string | null>(null)
  // Dar pencerede (styles.css'teki 900 px çekmece kırılımı) kenar çubuğu ikon şeridine iner; eşik aşılınca yeniden açılır.
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => window.matchMedia(NARROW_QUERY).matches)
  useEffect(() => {
    const mq = window.matchMedia(NARROW_QUERY)
    const onChange = (e: MediaQueryListEvent) => setSidebarCollapsed(e.matches)
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])
  const [panelOpen, setPanelOpen] = useState(false)
  // Kapanırken panel `--kor-dur-panel` boyunca çizili kalır (çıkış animasyonu, §23.1).
  const panelShown = usePresence(panelOpen, 'panel')
  // Kullanıcı değişiklik panelini kapattıysa, aynı turun sonraki dosya değişikliklerinde tekrar açılmasın (tur başında sıfırlanır).
  const panelDismissedRef = useRef(false)
  const [promptDialogOpen, setPromptDialogOpen] = useState(false)
  const [memoryDialogOpen, setMemoryDialogOpen] = useState(false)
  const [usageRefreshKey, setUsageRefreshKey] = useState(0)
  const [shortcutsOpen, setShortcutsOpen] = useState(false) // cdesktop klavye kısayolları penceresi

  // [osint] §27.10 macOS Handoff: kayıtlı açık oturum diğer Apple cihazlarına bildirilir (diğer OS'lerde işlem yok).
  const handoffTitle = sessions?.find((s) => s.id === activeId)?.title ?? null
  const handoffKnown = sessions?.some((s) => s.id === activeId) ?? false
  useEffect(() => {
    const url = handoffKnown ? `kor://resume?id=${encodeURIComponent(activeId)}` : null
    void osextraApi.handoff(url, handoffTitle).catch(() => false)
  }, [activeId, handoffKnown, handoffTitle])

  const chat = store[activeId] ?? initialChatState
  // Olay dinleyicisi bir kez kurulur; aktif sohbet ref ile okunur (yeniden abonelikte olay kaçmasın).
  const activeIdRef = useRef(activeId)
  activeIdRef.current = activeId
  const openSeqRef = useRef(0)
  const dispatch = useCallback((action: ChatAction) => dispatchStore({ sessionId: activeId, action }), [activeId])
  const changedPaths = useMemo(() => new Set(chat.changes.map((c) => c.path)), [chat.changes])
  // cdesktop mesaj kutusu şeridi: oturum toplam +/- (yan panelle aynı sayım)
  const changeTotals = useMemo(() => chat.changes.reduce((acc, c) => { const s = c.lazy ?? lineStats(c.before, c.after); return { added: acc.added + s.added, removed: acc.removed + s.removed } }, { added: 0, removed: 0 }), [chat.changes])

  useEffect(() => {
    // [perf] Token başına dispatch yerine kare başına bir: metin parçaları birleşir, diğer olaylar sırayı korur.
    const stream = createStreamCoalescer((sessionId, event, at) => dispatchStore({ sessionId, action: { type: 'event', event, at } }))
    const stop = api.onEvent((event) => {
        const { sessionId, ...body } = event
        stream.push(sessionId, body, Date.now())
        if (sessionId !== activeIdRef.current && (body.type === 'done' || body.type === 'error' || body.type === 'confirm')) {
          setUnread((prev) => new Set(prev).add(sessionId))
        }
        // Ses bildirimi artık "beni çağır" kuralıyla Rust'tan gelir (ADR 0051, useNotificationSound).
        if (sessionId === activeIdRef.current && body.type === 'fileChanged' && !panelDismissedRef.current) setPanelOpen(true)
        if (body.type === 'turnStart') panelDismissedRef.current = false // yeni turda otomatik açılma tekrar etkin
        if (body.type === 'fileChanged' || body.type === 'done') void mutate(isFsKey)
        if (body.type === 'turnStart' || body.type === 'done' || body.type === 'error') {
          if (body.type !== 'turnStart') setUsageRefreshKey((k) => k + 1)
          // Tur başında ve sonunda sohbet diske yazılır: geçmiş listesi yenilenir.
          void mutate('sessions')
          void mutate('state')
        }
    })
    return () => {
      stop()
      stream.flush()
    }
  }, [mutate])

  // Kilitlenme/yeni pencere sonrası: süren turlar ve bekleyen onay/soru kartları geri gelir (abonelikten sonra).
  useEffect(() => {
    void api.liveSessions().then(
      (list) => {
        for (const live of list) for (const action of liveActions(live)) dispatchStore({ sessionId: live.sessionId, action })
      },
      (err: unknown) => console.warn('Süren sohbetler alınamadı:', err),
    )
  }, [])

  const statusOf = useCallback(
    (id: string): SessionStatus => {
      const s = store[id]
      if (s?.items.some((i) => i.kind === 'tool' && i.awaitingApproval)) return 'waiting'
      if (s?.running || appState?.runningSessionIds.includes(id)) return 'running'
      if (unread.has(id)) return 'unread'
      return 'idle'
    },
    [store, unread, appState?.runningSessionIds],
  )

  // [g1] kancalar (hook) — erken return öncesi
  const g1 = useSessionExtras({
    hasApiKey: appState?.hasApiKey === true,
    modelsLoaded: models !== undefined,
    sessions,
    store,
    activeId,
    unread,
    setUnread,
    dispatchStore,
    mutate,
  })

  // [g2] kancalar (hook) — erken return öncesi
  const g2 = useG2App({ activeId, dispatchStore, draft, setDraft, attachments, setAttachments })

  // [g3] kancalar (hook) — erken return öncesi

  // [g4] kancalar (hook) — erken return öncesi
  const g4 = useG4({ activeId, chat, root: appState?.root ?? null, dispatchStore, setAttachments, setDraft })

  // [g5] kancalar (hook) — erken return öncesi

  // [g6] kancalar (hook) — erken return öncesi

  // [g9] kancalar (hook) — erken return öncesi

  // [g10] kancalar (hook) — erken return öncesi

  // [g11] kancalar (hook) — erken return öncesi

  // [g12] kancalar (hook) — erken return öncesi

  // [g13] kancalar (hook) — erken return öncesi

  // [g14] kancalar (hook) — erken return öncesi

  // [g15] kancalar (hook) — erken return öncesi

  // [g16] kancalar (hook) — erken return öncesi
  useG16Memory({ store, activeId, splitId: g1.splitId, runningIds: appState?.runningSessionIds, dispatchStore })

  // [g18] kancalar (hook) — erken return öncesi

  // [g19] kancalar (hook) — erken return öncesi

  // [g20] kancalar (hook) — erken return öncesi

  // [g21] kancalar (hook) — erken return öncesi
  useG21DraftInsert(setDraft) // belgeden alıntı / grafik bloğu mesaj kutusuna eklenir

  // [g22] kancalar (hook) — erken return öncesi

  // [g7] kancalar (hook) — erken return öncesi

  // [perf] Boşta kalan sohbet transkriptleri 5 dk sonra store'dan düşürülür: etkin, bölünmüş, çalışan ve okunmamış
  // olanlar kalır. Tekrar açılınca diskten yüklenir (openSession zaten store'da yoksa yükler).
  const lastSeenRef = useRef<Map<string, number>>(new Map())
  useEffect(() => {
    // Ayrılırken de işaretlenir: uzun süre açık kalan sohbet ayrılır ayrılmaz düşürülmesin.
    const stamp = () => { for (const id of [activeId, ...g1.splitIds]) lastSeenRef.current.set(id, Date.now()) }
    stamp()
    return stamp
  }, [activeId, g1.splitIds])
  useEffect(() => {
    const EVICT_MS = 5 * 60_000
    const timer = setInterval(() => {
      const now = Date.now()
      const keep = new Set<string>([activeIdRef.current, ...g1.splitIds])
      for (const id of Object.keys(store)) {
        const chat = store[id]
        // Değişiklik listesi ve geri al düğmeleri dökümden geri gelmez; süren araçlar da kaybolmamalı.
        if (keep.has(id) || !chat || chat.running || chat.changes.length > 0 || chat.items.some(isLiveItem) || unread.has(id) || appState?.runningSessionIds.includes(id)) continue
        const seen = lastSeenRef.current.get(id) ?? (lastSeenRef.current.set(id, now), now)
        if (now - seen > EVICT_MS) {
          dispatchStore({ sessionId: id, action: { type: 'evict' } })
          lastSeenRef.current.delete(id)
        }
      }
    }, 60_000)
    return () => clearInterval(timer)
  }, [store, unread, g1.splitIds, appState?.runningSessionIds])

  // [cdx] Ajan tarayıcıyı kullanmaya başlayınca Tarayıcı paneli açılır (kullanıcı ne yapıldığını görür).
  useEffect(
    () =>
      api.abrowser.onEvent((ev) => {
        if (ev.type !== 'open') return
        panelDismissedRef.current = false
        setPanelOpen(true)
        requestAnimationFrame(() => window.dispatchEvent(new CustomEvent('panel:view', { detail: 'browser' })))
      }),
    [],
  )
  // [cdx] Ajan preview aracıyla "göster" deyince panel Web önizleme görünümüyle açılır (erken return öncesi).
  useEffect(
    () =>
      api.preview.onEvent((ev) => {
        if (ev.type !== 'open') return
        panelDismissedRef.current = false
        setPanelOpen(true)
        requestAnimationFrame(() => window.dispatchEvent(new CustomEvent('panel:view', { detail: 'devpreview' })))
      }),
    [],
  )

  // ADR 0051: tepsi, bildirim ve CLI'dan gelen kabuk komutları; işleyiciler aşağıda tanımlanınca bağlanır.
  const shellHandlers = useRef<ShellHandlers | null>(null)
  useShellBridge(shellHandlers)
  useActivityReporter()
  useNotificationSound()

  if (stateError) return <main className="center-screen error-text" role="alert">{errorMessage(stateError)}</main>
  if (!appState) return <main className="center-screen muted">Yükleniyor...</main>
  // İlk açılış hesap ya da anahtar istemez: uygulama açılır, ana ekran sağlayıcı bağlamayı önerir.

  // Tura bağlı olmayan hatalar çalışan turu kapatmaz; yalnızca gönderim hataları 'failed' olur.
  const report = (err: unknown) => dispatch({ type: 'notify', message: errorMessage(err) })

  const resetComposer = () => {
    setAttachments([])
    setDraft('')
    setPreviewPath(null)
    setPanelOpen(false)
  }

  const newChat = async () => {
    // Çalışan sohbet durdurulmaz: arka planda devam eder, kenar çubuğundan geri dönülebilir.
    setActiveId(crypto.randomUUID())
    resetComposer()
    await api.setActiveSession(null)
  }

  const openFolder = async () => {
    try {
      const root = await api.openWorkspace()
      if (!root) return
      await newChat()
      await refreshState()
      await mutate(isFsKey)
    } catch (err) {
      report(err)
    }
  }

  const switchWorkspace = async (fn: () => Promise<unknown>) => {
    try {
      await fn()
      await newChat()
      await refreshState()
      await mutate(isFsKey)
    } catch (err) {
      report(err)
    }
  }
  // §21.1, §25.3: pencerede yazma alanı dışına bırakılan tek klasör proje olarak açılır; dosyalar eklere girer.
  dropOutsideRef.current = (paths) => {
    const add = () => setAttachments((prev) => [...new Set([...prev, ...paths])])
    if (paths.length !== 1) return add()
    void api
      .setWorkspacePath(paths[0]!)
      .then(async () => {
        await newChat()
        await refreshState()
        await mutate(isFsKey)
      })
      .catch(add)
  }
  const clearFolder = () => void switchWorkspace(() => api.clearWorkspace())
  const pickRecentFolder = (path: string) => void switchWorkspace(() => api.setWorkspacePath(path))

  const openSession = async (id: string) => {
    const seq = ++openSeqRef.current
    const transcript = await api.openSession(id)
    if (seq !== openSeqRef.current) return // daha yeni bir sohbet açıldı; eski yanıt etkin sohbeti değiştirmesin
    // Bu oturumun olaylarını zaten izliyorsak (arka planda çalışıyor olabilir) mevcut durum korunur.
    if (needsTranscript(store[id])) {
      const live = (await api.liveSessions()).find((l) => l.sessionId === id)
      if (seq !== openSeqRef.current) return
      if (live) for (const action of liveActions(live)) dispatchStore({ sessionId: id, action })
      dispatchStore({ sessionId: id, action: { type: 'load', transcript, ...(live?.since != null ? { runningSince: live.since } : {}) } })
    }
    setActiveId(id)
    setUnread((prev) => {
      const next = new Set(prev)
      next.delete(id)
      return next
    })
    resetComposer()
    await refreshState()
    await mutate(isFsKey)
  }

  const deleteSession = async (id: string) => {
    await api.deleteSession(id)
    if (id === activeId) await newChat()
    await Promise.all([refreshSessions(), refreshState()])
  }

  const pinSession = async (id: string, pinned: boolean) => {
    await api.pinSession(id, pinned)
    await refreshSessions()
  }


  // Otomatik bağlam: yan panelde önizlenen dosya modele bildirilir (panel kapalıysa yok sayılır).
  const openFileContext = panelOpen ? previewPath : null

  const send = (raw: string) => {
    const sessionId = activeId
    // cdesktop: diff'e eklenen inceleme yorumları mesajın başına biçimli eklenir ve temizlenir.
    const text = mergePreviewIntoMessage(mergeReviewIntoMessage(raw, getReviewComments()), getPreviewSelections())
    clearReviewComments()
    clearPreviewSelections()
    dispatchStore({ sessionId, action: { type: 'userSent', text, attachments, at: Date.now() } })
    setAttachments([])
    setDraft('')
    api
      .setActiveSession(sessionId)
      .then(() => api.send(sessionId, text, attachments, openFileContext))
      .catch((err: unknown) => dispatchStore({ sessionId, action: { type: 'failed', message: errorMessage(err) } }))
  }

  const approve = (id: string, approval: Approval, files?: FileDecisions) => {
    dispatch({ type: 'approved', id })
    void approveWithFiles(id, approval, files) // [review] dosya bazında kararlar (§12.3)
  }

  const addAttachments = (paths: string[]) => setAttachments((prev) => [...new Set([...prev, ...paths])])

  const pickFiles = async () => {
    try {
      addAttachments(await api.pickFiles())
    } catch (err) {
      report(err)
    }
  }

  const revert = async (path: string) => {
    await api.revertFile(activeId, path)
    dispatch({ type: 'reverted', path })
    await mutate(isFsKey)
  }

  const changeModel = async (model: string) => {
    await api.setModel(model)
    await refreshState()
  }

  const changeMode = async (mode: PermissionMode) => {
    await api.setPermissionMode(mode)
    await refreshState()
  }

  const makeDefaultMode = async (mode: PermissionMode) => {
    await api.setDefaultPermissionMode(mode)
    await refreshState()
  }

  // [g3] line: dosya yolu bağlantılarından gelen satır numarası; [g4] önizleme o satıra kaydırıp vurgular
  const openFile = (path: string, line?: number) => {
    setPreviewPath(path)
    setPanelOpen(true)
    panelDismissedRef.current = false // kullanıcı açtı
    g4.setLine(line) // [g4] satıra git (line verilmezse vurgu kalkar)
  }

  // [g1] işleyiciler
  g1.openRef.current = openSession
  // §27.3 (erken return'lerden sonra tanımlandıkları için ref ile)
  forkOpenRef.current = openSession
  forkReportRef.current = report
  const g1SidebarProps = g1.sidebarProps(report)

  // cdesktop komut paleti (Ctrl/⌘+K) eylemleri
  const paletteActions: PaletteActions = {
    newChat: () => void newChat().catch(report),
    openFolder: () => void openFolder(),
    copyPath: () => { if (appState.root) void navigator.clipboard.writeText(appState.root) },
    openPanel: (view) => {
      panelDismissedRef.current = false
      setPanelOpen(true)
      requestAnimationFrame(() => window.dispatchEvent(new CustomEvent('panel:view', { detail: view })))
    },
    openSettings: () => openG7Dialog({ kind: 'settings' }),
    openSession: (id) => void openSession(id).catch(report),
    showShortcuts: () => setShortcutsOpen(true),
    toggleSidebar: () => setSidebarCollapsed((c) => !c),
    archiveCurrent: () => { const s = sessions?.find((x) => x.id === activeId); if (s) g1SidebarProps.onArchiveSession(s.id, !s.archived) },
    pinCurrent: () => { const s = sessions?.find((x) => x.id === activeId); if (s) void pinSession(s.id, !s.pinned).catch(report) },
    deleteCurrent: () => { if (sessions?.some((x) => x.id === activeId)) void confirmAction({ title: 'Sohbet silinsin mi?', message: 'Bu sohbet kalıcı olarak silinecek.', confirmLabel: 'Sil', danger: true }).then((ok) => { if (ok) void deleteSession(activeId).catch(report) }) },
    duplicateCurrent: () => { if (sessions?.some((x) => x.id === activeId)) g1SidebarProps.onDuplicateSession(activeId, false) },
    renameCurrent: () => window.dispatchEvent(new CustomEvent('cdx:rename', { detail: activeId })),
    nextSession: () => { const list = (sessions ?? []).filter((x) => !x.archived); const i = list.findIndex((x) => x.id === activeId); const n = list[i + 1] ?? list[0]; if (n && n.id !== activeId) void openSession(n.id).catch(report) },
    prevSession: () => { const list = (sessions ?? []).filter((x) => !x.archived); const i = list.findIndex((x) => x.id === activeId); const n = i > 0 ? list[i - 1] : list.at(-1); if (n && n.id !== activeId) void openSession(n.id).catch(report) },
    gitPush: () => void api.git.push().then(() => mutate('git:status')).catch(report),
    gitMerge: () => void api.git.status().then((st) => api.git.merge(st.defaultBase ?? 'main')).then(() => mutate('git:status')).catch(report),
    gitRebase: () => void api.git.status().then((st) => api.git.rebase(st.defaultBase ?? 'main')).then(() => mutate('git:status')).catch(report),
    exportCurrent: () => { if (sessions?.some((x) => x.id === activeId)) g1SidebarProps.onExportSession(activeId) },
    gitPr: () => { panelDismissedRef.current = false; setPanelOpen(true); requestAnimationFrame(() => { window.dispatchEvent(new CustomEvent('panel:view', { detail: 'git' })); window.dispatchEvent(new Event('cdx:git:pr')) }) },
  }
  openPanelRef.current = (view) => paletteActions.openPanel(view as Parameters<PaletteActions['openPanel']>[0])
  shellHandlers.current = {
    openSession: (id) => void openSession(id).catch(report),
    openPanel: (view) => {
      if (view === 'approvals') window.dispatchEvent(new Event(OPEN_QUEUE_EVENT))
      else paletteActions.openPanel(view)
    },
    openProject: (path) => pickRecentFolder(path),
    newChat: () => void newChat().catch(report),
    answered: (sessionId, id) => dispatchStore({ sessionId, action: { type: 'approved', id } }),
    openUpdate: () => openG16Panel(),
  }
  const g1SplitId = g1.splitId
  const g1SplitIds = g1.splitIds

  // [g2] işleyiciler
  g2.bind({
    newChat: () => void newChat().catch(report),
    openMemory: () => setMemoryDialogOpen(true),
    openSystemPrompt: () => setPromptDialogOpen(true),
    pickFiles: () => void pickFiles(),
  })

  // [g3] işleyiciler
  // Akış kartları (öneri düğmeleri, yeniden dene, dosya bağlantıları, soru/plan cevapları) App'e bu köprüyle ulaşır.
  g3Actions.current = {
    send: (text, files = []) => {
      const sessionId = activeId
      dispatchStore({ sessionId, action: { type: 'userSent', text, attachments: files, at: Date.now() } })
      api
        .setActiveSession(sessionId)
        .then(() => api.send(sessionId, text, files, openFileContext))
        .catch((err: unknown) => dispatchStore({ sessionId, action: { type: 'failed', message: errorMessage(err) } }))
    },
    openFile,
    openChanges: () => {
      setPreviewPath(null)
      setPanelOpen(true)
      panelDismissedRef.current = false
    },
    answer: async (callId, answer) => {
      await api.g3Answer(activeId, callId, answer)
      // Plan onayı izin modunu değiştirir; mod menüsü güncellensin.
      if (answer.kind === 'planApprove') await refreshState()
    },
    refreshFiles: () => void mutate(isFsKey),
    times: chat.g3?.times ?? {},
  }

  // [g4] işleyiciler

  // [g5] işleyiciler

  // [g6] işleyiciler

  // [g9] işleyiciler

  // [g10] işleyiciler

  // [g11] işleyiciler

  // [g12] işleyiciler

  // [g13] işleyiciler

  // [g14] işleyiciler

  // [g15] işleyiciler

  // [g16] işleyiciler

  // [g18] işleyiciler

  // [g19] işleyiciler

  // [g20] işleyiciler

  // [g21] işleyiciler

  // [g22] işleyiciler

  // [g7] işleyiciler
  // Ağ hatasıyla biten tur ana süreçte geri alınır (kullanıcı mesajı dahil); aynı mesaj ve ekler yeniden gönderilir.
  const retryLastTurn = () => {
    const lastUser = chat.items.findLast((i) => i.kind === 'user')
    if (!lastUser || lastUser.kind !== 'user' || chat.running) return
    const sessionId = activeId
    dispatchStore({ sessionId, action: { type: 'userSent', text: lastUser.text, attachments: lastUser.attachments, at: Date.now() } })
    api.send(sessionId, lastUser.text, lastUser.attachments).catch((err: unknown) => dispatchStore({ sessionId, action: { type: 'failed', message: errorMessage(err) } }))
  }

  const composer = (
    <Composer
      draft={draft}
      onDraftChange={setDraft}
      running={chat.running}
      disabled={!appState.hasApiKey}
      attachments={attachments}
      root={appState.root}
      recentWorkspaces={appState.recentWorkspaces}
      onOpenFolder={() => void openFolder()}
      onClearFolder={clearFolder}
      onPickRecentFolder={pickRecentFolder}
      onPickFiles={() => void pickFiles()}
      onRemoveAttachment={(path) => setAttachments((prev) => prev.filter((p) => p !== path))}
      onSend={send}
      onAbort={() => void api.abort(activeId)}
      model={appState.model}
      models={models}
      modelsError={modelsError ? errorMessage(modelsError) : null}
      onModelChange={(id) => void changeModel(id).catch(report)}
      permissionMode={appState.permissionMode}
      defaultPermissionMode={appState.defaultPermissionMode}
      onPermissionModeChange={(mode) => void changeMode(mode).catch(report)}
      onMakeDefaultMode={(mode) => void makeDefaultMode(mode).catch(report)}
      usageRefreshKey={usageRefreshKey}
      sessionId={store[activeId] ? activeId : null}
      statusRow={
        chat.items.length > 0 ? (
          <ComposerStatusRow
            files={chat.changes.length}
            added={changeTotals.added}
            removed={changeTotals.removed}
            todos={latestTodos(chat.items, chat.running)}
            running={chat.running}
            sessions={(sessions ?? []).filter((s) => !s.archived && s.root === appState.root)}
            activeId={activeId}
            onOpenPanel={() => { panelDismissedRef.current = false; setPanelOpen(true) }}
            onOpenSession={(id) => void openSession(id).catch(report)}
            onNewChat={() => void newChat().catch(report)}
          />
        ) : null
      }
    />
  )

  const backgroundRunning = appState.runningSessionIds.filter((id) => id !== activeId).length

  return (
    <div className="app">
      <TitleBar />
      <div className="app-body">
        <Sidebar
        collapsed={sidebarCollapsed}
        onToggle={() => setSidebarCollapsed((c) => !c)}
        root={appState.root}
        selected={previewPath}
        changed={changedPaths}
        sessions={sessions}
        sessionsError={sessionsError ? errorMessage(sessionsError) : null}
        activeSessionId={activeId}
        statusOf={statusOf}
        onNewChat={() => void newChat().catch(report)}
        onOpenFolder={() => void openFolder()}
        onOpenFile={openFile}
        onOpenSession={(id) => void openSession(id).catch(report)}
        onPinSession={(id, pinned) => void pinSession(id, pinned).catch(report)}
        onDeleteSession={(id) => void deleteSession(id).catch(report)}
        onOpenMemory={() => setMemoryDialogOpen(true)}
        onOpenSystemPrompt={() => setPromptDialogOpen(true)}
        {...g1SidebarProps}
      />

      <div className="session-grid" data-panes={1 + g1SplitIds.length}>
      <main className="main">
        <header className="main-header">
          <span className="header-path" title={appState.root ? displayPath(appState.root) : undefined}>{appState.root ? displayPath(appState.root) : ''}</span>
          {backgroundRunning > 0 && (
            <span className="bg-pill" role="status">
              <span className="bg-dot" aria-hidden="true" />
              {backgroundRunning} sohbet arka planda çalışıyor
            </span>
          )}
          <Tooltip side="bottom" text="Değişiklikler">
            <button
              type="button"
              className={`action-btn header-toggle${panelOpen ? ' active' : ''}`}
              onClick={() => setPanelOpen((o) => { panelDismissedRef.current = o; return !o })}
              aria-label={panelOpen ? 'Değişiklik panelini kapat' : 'Değişiklik panelini aç'}
              aria-pressed={panelOpen}
            >
              <PanelRight size={16} aria-hidden="true" />
              {chat.changes.length > 0 && <span className="count">{chat.changes.length}</span>}
            </button>
          </Tooltip>
          {/* cdesktop üst sağ araç çubuğu: ☰ komut paleti · ? kısayollar · ⚙ ayarlar */}
          <span className="cdx-header-sep" aria-hidden="true" />
          <Tooltip side="bottom" text="Komut paleti (Ctrl+K)">
            <button type="button" className="action-btn" onClick={() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }))} aria-label="Komut paleti (Ctrl+K)">
              <Keyboard size={16} aria-hidden="true" />
            </button>
          </Tooltip>
          <Tooltip side="bottom" text="Klavye kısayolları (?)">
            <button type="button" className="action-btn" onClick={() => setShortcutsOpen(true)} aria-label="Klavye kısayolları">
              <HelpCircle size={16} aria-hidden="true" />
            </button>
          </Tooltip>
          <Tooltip side="bottom" text="Ayarlar (Ctrl+,)">
            <button type="button" className="action-btn" onClick={() => openG7Dialog({ kind: 'settings' })} aria-label="Ayarlar">
              <SettingsIcon size={16} aria-hidden="true" />
            </button>
          </Tooltip>
        </header>
        {/* cdesktop dikey eylem şeridi (sohbet alanının sağ kenarı) */}
        {chat.items.length > 0 && <FloatingRail root={appState.root} onOpenPanel={paletteActions.openPanel} onOpenFolder={() => void openFolder()} />}

        {chat.items.length === 0 ? (
          <HomeEmpty
            composer={composer}
            onSuggestion={setDraft}
            disabled={!appState.hasApiKey}
            root={appState.root}
            onOpenFolder={() => void openFolder()}
            onOpenSample={() => void webviewApi.sampleProject().then((path) => pickRecentFolder(path)).catch(report)}
          />
        ) : (
          <Thread
            key={activeId}
            sessionId={activeId}
            model={appState.model}
            items={chat.items}
            running={chat.running}
            turnStartedAt={chat.turnStartedAt}
            composer={composer}
            onApprove={approve}
            renderUserActions={g4.userActions}
            onSaveSummary={async (_foldId, body) => {
              try {
                return await api.editCompactionSummary(activeId, body)
              } catch (err) {
                // Kaydetme başarısızsa düzenleme açık kalsın ve kullanıcı nedeni görsün.
                dispatchStore({ sessionId: activeId, action: { type: 'notify', message: errorMessage(err) } })
                return false
              }
            }}
          />
        )}
      </main>

      {/* [g1] cdesktop çoklu oturum ızgarası: ana oturum + açık yan bölmeler */}
      {g1SplitIds.map((id) => (
        <SplitPane
          key={id}
          sessionId={id}
          chat={store[id] ?? initialChatState}
          usageRefreshKey={usageRefreshKey}
          dispatch={(action) => dispatchStore({ sessionId: id, action })}
          onClose={() => g1.closeSplit(id)}
        />
      ))}
      </div>

      {/* Yan panel yatay düzenin (.app-body) içinde: .app dikeydir (başlık çubuğu + gövde);
          dışarıda kalırsa panel sohbetin ALTINA düşüp sohbeti sıfır yüksekliğe ezer. */}
      {panelShown && (
        <SidePanel
          key={`${activeId}:${previewPath ?? 'none'}`}
          closing={!panelOpen}
          changes={chat.changes}
          previewPath={previewPath}
          onAttach={(path) => addAttachments([path])}
          onRevert={revert}
          onClose={() => { setPanelOpen(false); panelDismissedRef.current = true }}
          sessionId={store[activeId] ? activeId : null}
          onHydrateChange={(path, before, after) => dispatchStore({ sessionId: activeId, action: { type: 'hydrateChange', path, before, after } })}
          g4={g4.panel(openFile)} // [g4]
        />
      )}
      </div>

      {/* [g2] diyaloglar ve katmanlar */}

      {/* [g3] diyaloglar ve katmanlar */}

      {/* [g4] diyaloglar ve katmanlar */}
      {g4.dialogs}
      <ConfirmHost />
      <SystemOverlay openSystem={() => paletteActions.openPanel('system')} />
      {/* [osint] sağ tık menüsü, kor:// bağlantıları, atlama listesi, global kısayol, pencereye bırakılan klasör (ADR 0052) */}
      <OsintHost
        root={appState.root}
        openProject={(root) => switchWorkspace(() => api.setWorkspacePath(root))}
        attach={addAttachments}
        send={send}
        draft={setDraft}
        resume={openSession}
        newChat={newChat}
        showRunning={() => setSidebarCollapsed(false)}
        report={report}
      />
      <BatchEditDialog root={appState.root} />
      {/* cdesktop komut paleti + klavye kısayolları */}
      <CommandPalette actions={paletteActions} sessions={sessions ?? []} />
      {/* §21 WebView kilidi: Kor bağlam menüsü, kapatma diyaloğu, yazı boyutu, bildirimler (ADR 0053) */}
      <KorChrome />
      {shortcutsOpen && <ShortcutsDialog onClose={() => setShortcutsOpen(false)} />}

      {/* [g5] diyaloglar ve katmanlar */}

      {/* [g6] diyaloglar ve katmanlar */}
      <ProjectMapPrompt root={appState.root} />

      {/* [g9] diyaloglar ve katmanlar */}

      {/* [g10] diyaloglar ve katmanlar */}
      <G10Layer activeId={activeId} chat={chat} dispatchStore={dispatchStore} onOpenSession={openSession} />

      {/* [g11] diyaloglar ve katmanlar */}
      <G11Layer sessionId={activeId} onOpenGlobalEditor={() => setMemoryDialogOpen(true)} />
      <G30MemoryLayer />

      {/* [g12] diyaloglar ve katmanlar */}
      <BulkDeleteHost activeSessionId={activeId} />

      {/* [g13] diyaloglar ve katmanlar */}
      <G13Layer sessionId={activeId} root={appState.root} setDraft={setDraft} openFile={openFile} openGlobalPrompt={() => setPromptDialogOpen(true)} />

      {/* [g14] diyaloglar ve katmanlar */}
      <G14Layer sessionId={store[activeId] ? activeId : null} onModelChange={(id) => void changeModel(id).catch(report)} />

      {/* [g15] diyaloglar ve katmanlar */}

      {/* [g16] diyaloglar ve katmanlar */}
      <G16Panel />

      {/* [g18] diyaloglar ve katmanlar */}
      <G18PasteChart />

      {/* [g19] diyaloglar ve katmanlar */}

      {/* [g20] diyaloglar ve katmanlar */}
      <PrefsLayer sessionId={activeId} />
      <G20Layer />

      {/* [g21] diyaloglar ve katmanlar */}

      {/* [g22] diyaloglar ve katmanlar */}

      {/* [g7] diyaloglar ve katmanlar */}
      <G7Layer
        root={appState.root}
        currentModel={appState.model}
        items={chat.items}
        running={chat.running}
        onRetry={retryLastTurn}
        onModelChange={(id) => void changeModel(id).catch(report)}
        onOpenMemory={() => setMemoryDialogOpen(true)}
        onOpenSystemPrompt={() => setPromptDialogOpen(true)}
        onSessionsCleared={() => void newChat().catch(report)}
      />

      <SystemPromptDialog open={promptDialogOpen} onClose={() => setPromptDialogOpen(false)} />
      <MemoryDialog open={memoryDialogOpen} onClose={() => setMemoryDialogOpen(false)} />
    </div>
  )
}
