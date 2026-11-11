// Grup 1 arayüz durumu: bölümler, fiyatlar, yan bölme, son sohbetin geri yüklenmesi, pencere odağı eşitleme.
import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import useSWR, { type ScopedMutator } from 'swr'
import type { SessionMeta } from '../ipc.ts'
import { api, errorMessage } from './api.ts'
import { liveActions, needsTranscript, type ChatAction, type ChatStore } from './chat-state.ts'
import type { SessionExtras } from './SessionList.tsx'

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const isFsKey = (key: unknown) => Array.isArray(key) && (key[0] === 'dir' || key[0] === 'file')

/** Yeni pencerede açılan sohbet (`?session=<id>`); ana pencerede null. */
function sessionFromUrl(): string | null {
  const value = new URLSearchParams(window.location.search).get('session')
  return value && UUID.test(value) ? value : null
}

interface Options {
  hasApiKey: boolean
  modelsLoaded: boolean
  sessions: SessionMeta[] | undefined
  store: ChatStore
  activeId: string
  unread: ReadonlySet<string>
  setUnread: Dispatch<SetStateAction<ReadonlySet<string>>>
  dispatchStore: (action: { sessionId: string; action: ChatAction }) => void
  mutate: ScopedMutator
}

export function useSessionExtras(o: Options) {
  const { hasApiKey, sessions, store, activeId, unread, setUnread, dispatchStore, mutate } = o
  const { data: sections, error: sectionsError } = useSWR(hasApiKey ? 'g1:sections' : null, () => api.listSections())
  const { data: pricing } = useSWR(o.modelsLoaded ? 'g1:pricing' : null, () => api.getModelPricing(), { revalidateOnFocus: false })
  // cdesktop çoklu oturum ızgarası: ana oturum + en fazla 3 yan bölme (toplam 4 pane).
  const MAX_SPLITS = 3
  const [splitIds, setSplitIds] = useState<string[]>([])
  const splitId = splitIds[0] ?? null
  const setSplitId = (id: string | null) => setSplitIds(id ? [id] : [])
  const closeSplit = (id: string) => setSplitIds((prev) => prev.filter((x) => x !== id))
  const [windowSession] = useState(sessionFromUrl)
  const [restoreDone, setRestoreDone] = useState(false)
  /** App, openSession'ı her çizimde buraya yazar (erken return'lerden sonra tanımlandığı için). */
  const openRef = useRef<((id: string) => Promise<void>) | null>(null)
  const storeRef = useRef(store)
  storeRef.current = store
  const activeIdRef = useRef(activeId)
  activeIdRef.current = activeId
  const restoreStarted = useRef(false)
  const lastWritten = useRef<string | null | undefined>(undefined)

  useEffect(
    () =>
      api.onSessionsChanged(() => {
        void mutate('sessions')
        void mutate('g1:sections')
      }),
    [mutate],
  )

  // Açılışta: yeni pencerede istenen sohbet, ana pencerede son açık sohbet. Açılamazsa boş başlanır.
  useEffect(() => {
    if (!hasApiKey || restoreStarted.current) return
    restoreStarted.current = true
    const restore = async () => {
      const id = windowSession ?? (await api.getLastSession())
      if (id && openRef.current) await openRef.current(id)
    }
    restore()
      .catch((err: unknown) => {
        console.warn('Sohbet geri yüklenemedi; boş başlanıyor:', errorMessage(err))
        if (windowSession) dispatchStore({ sessionId: activeIdRef.current, action: { type: 'failed', message: errorMessage(err) } })
      })
      .finally(() => setRestoreDone(true))
  }, [hasApiKey, windowSession, dispatchStore])

  // Ana pencerede etkin sohbet kaydedilir (taslak sohbette null); ek pencereler bu kaydı değiştirmez.
  useEffect(() => {
    if (windowSession || !restoreDone || !sessions) return
    const id = sessions.some((s) => s.id === activeId) ? activeId : null
    if (id === lastWritten.current) return
    lastWritten.current = id
    api.setLastSession(id).catch((err: unknown) => console.error('Son sohbet kaydedilemedi:', errorMessage(err)))
  }, [activeId, sessions, restoreDone, windowSession])

  // Birden çok pencerede ana süreçteki etkin sohbet/klasör, odaklanan pencerenin sohbetine eşitlenir.
  useEffect(() => {
    const onFocus = () => {
      const id = activeIdRef.current
      if (!storeRef.current[id]) return // Taslak sohbet: ana süreçte henüz yok.
      api
        .activateSession(id)
        .then((changed) => {
          if (!changed) return
          void mutate('state')
          void mutate(isFsKey)
        })
        .catch((err: unknown) => console.error('Sohbet etkinleştirilemedi:', errorMessage(err)))
    }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [mutate])

  // Yan bölmedeki sohbet ana bölmede açıldıysa veya silindiyse bölme kapanır; görünür olduğu için okunmamış sayılmaz.
  useEffect(() => {
    if (splitIds.length === 0) return
    // Etkin olan veya artık var olmayan bölmeler kapanır.
    const valid = splitIds.filter((id) => id !== activeId && (!sessions || sessions.some((s) => s.id === id)))
    if (valid.length !== splitIds.length) {
      setSplitIds(valid)
      return
    }
    // Görünür bölmeler okunmamış sayılmaz.
    if (splitIds.some((id) => unread.has(id))) {
      setUnread((prev) => {
        const next = new Set(prev)
        for (const id of splitIds) next.delete(id)
        return next
      })
    }
  }, [splitIds, activeId, sessions, unread, setUnread])

  const openSplit = async (id: string) => {
    if (id === activeIdRef.current || splitIds.includes(id)) return
    const transcript = await api.loadSessionView(id)
    // Arka planda çalışan sohbetin canlı durumu korunur.
    if (needsTranscript(storeRef.current[id])) {
      const live = (await api.liveSessions()).find((l) => l.sessionId === id)
      if (live) for (const action of liveActions(live)) dispatchStore({ sessionId: id, action })
      dispatchStore({ sessionId: id, action: { type: 'load', transcript, ...(live?.since != null ? { runningSince: live.since } : {}) } })
    }
    setSplitIds((prev) => (prev.includes(id) ? prev : [...prev, id].slice(-MAX_SPLITS)))
  }

  /** Kenar çubuğu işlemleri; hatalar etkin sohbete hata satırı olarak düşer. */
  const sidebarProps = (report: (err: unknown) => void): SessionExtras => {
    const run = (p: Promise<unknown>) => void p.catch(report)
    return {
      sections,
      sectionsError: sectionsError ? errorMessage(sectionsError) : null,
      pricing,
      splitSessionId: splitId,
      onRenameSession: (id, title) => run(api.renameSession(id, title)),
      onArchiveSession: (id, archived) => run(api.setSessionArchived(id, archived)),
      onMoveSession: (id, sectionId) => run(api.moveSessionToSection(id, sectionId)),
      onOpenSplit: (id) => run(openSplit(id)),
      onOpenWindow: (id) => run(api.openSessionWindow(id)),
      onDuplicateSession: (id, spinOff) => run(api.duplicateSession(id, spinOff).then((newId) => openRef.current?.(newId))),
      onExportSession: (id) => run(api.exportSession(id)),
      // Bölüm oluşturma hatası satır içi gösterilir; bu yüzden çağırana döner.
      onCreateSection: (name) => api.createSection(name).then(() => undefined),
      onRenameSection: (id, name) => run(api.renameSection(id, name)),
      onDeleteSection: (id) => run(api.deleteSection(id)),
    }
  }

  return { splitId, splitIds, setSplitId, closeSplit, openRef, sidebarProps }
}
