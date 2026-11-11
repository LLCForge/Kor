// ipc.ts `shellui` ad alanını tiplemiyor; tipler ipc-shellui.ts içinde (ADR 0051).
import { useEffect } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { ShellState, ShellUiApi } from '../../ipc-shellui.ts'
import { api } from '../api.ts'
import { playNotificationSound } from '../notify-sound.ts'
import { autoSoundOn } from '../prefs/readability.ts' // [ADR 0050] `[auto] sound`

export const shellui = (api as unknown as { shellui: ShellUiApi }).shellui

export const STATE_KEY = 'shellui:state'
export const NOTES_KEY = 'shellui:notifications'

/** Kabuk durumu; Rust `kor:shellui/changed` yayınca tazelenir. */
export function useShellState() {
  const swr = useSWR<ShellState>(STATE_KEY, () => shellui.state())
  const { mutate } = swr
  useEffect(() => shellui.onChanged(() => void mutate()), [mutate])
  return swr
}

/** Bildirim listesi; yeni bildirim ya da değişiklikte tazelenir. */
export function useNotifications() {
  const swr = useSWR(NOTES_KEY, () => shellui.notifications())
  const { mutate } = swr
  useEffect(() => {
    const a = shellui.onNotification(() => void mutate())
    const b = shellui.onChanged(() => void mutate())
    return () => {
      a()
      b()
    }
  }, [mutate])
  return swr
}

/** §28.4 boşta kalma kuralı: tuş/fare girişi Rust'a en çok 5 sn'de bir bildirilir. */
export function useActivityReporter() {
  useEffect(() => {
    let last = 0
    const report = () => {
      const now = Date.now()
      if (now - last < 5000) return
      last = now
      void shellui.activity().catch(() => undefined)
    }
    const events = ['keydown', 'pointerdown', 'pointermove', 'wheel'] as const
    for (const e of events) window.addEventListener(e, report, { passive: true })
    report()
    return () => {
      for (const e of events) window.removeEventListener(e, report)
    }
  }, [])
}

/** Kural ses istediğinde, kullanıcının seçtiği bildirim sesi (Ayarlar → Bildirimler) çalar. */
export function useNotificationSound() {
  useEffect(
    () =>
      shellui.onNotification(({ alert }) => {
        if (!alert.sound) return
        void api
          .g7GetSettings()
          .then((s) => {
            if (s.notifications.sound && autoSoundOn()) playNotificationSound(s.notifications.soundName)
          })
          .catch(() => undefined)
      }),
    [],
  )
}

export interface ShellHandlers {
  openSession: (id: string) => void
  openPanel: (view: string) => void
  openProject: (path: string) => void
  newChat: () => void
  /** Onay tepsiden/CLI'dan yanıtlandı: sohbetteki kart kapanır. */
  answered: (sessionId: string, id: string) => void
  openUpdate: () => void
}

export const OPEN_TARGET_EVENT = 'kor:shell-open-target'

function onWindow<T>(name: string, fn: (detail: T) => void): () => void {
  const h = (e: Event) => fn((e as CustomEvent<T>).detail)
  window.addEventListener(name, h)
  return () => window.removeEventListener(name, h)
}

/** Rust'tan gelen kabuk komutlarını ana pencerenin işleyicilerine bağlar. Abonelik bir kez kurulur; işleyiciler
 * `ref` üzerinden okunur (ana pencere onları erken dönüşlerden sonra tanımlar). */
export function useShellBridge(ref: { readonly current: ShellHandlers | null }) {
  const { mutate } = useSWRConfig()
  useEffect(() => {
    const stops = [
      shellui.onOpenSession((id) => ref.current?.openSession(id)),
      shellui.onOpenPanel((v) => ref.current?.openPanel(v)),
      shellui.onOpenProject((p) => ref.current?.openProject(p)),
      shellui.onNewChat(() => ref.current?.newChat()),
      shellui.onAnswered((e) => {
        ref.current?.answered(e.sessionId, e.id)
        void mutate(STATE_KEY)
      }),
      shellui.onOpenUpdate(() => ref.current?.openUpdate()),
      onWindow(OPEN_TARGET_EVENT, (d: { sessionId: string | null; panel?: string }) => {
        if (d.sessionId) ref.current?.openSession(d.sessionId)
        if (d.panel === 'update') ref.current?.openUpdate()
        else if (d.panel) ref.current?.openPanel(d.panel)
      }),
    ]
    return () => {
      for (const s of stops) s()
    }
  }, [ref, mutate])
}
