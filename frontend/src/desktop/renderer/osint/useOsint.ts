// İşletim sisteminden gelen istekleri (sağ tık menüsü, kor:// bağlantıları, atlama listesi / Dock menüsü, global
// kısayol, pencereye bırakılan klasör) arayüzde uygular (ADR 0052). Adımlar saf `plan.ts`'te.
import { listen } from '@tauri-apps/api/event'
import { useEffect, useRef } from 'react'
import type { OsintApi, OsintRequest } from '../../ipc-osint.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { onNativeDrop } from '../kor-bridge.ts'
import { planRequest } from './plan.ts'

export const osintApi = (api as unknown as { osint: OsintApi }).osint

/** Toplu düzenleme penceresini açan pencere olayı (`BatchEditDialog` dinler). */
export const EDIT_EVENT = 'osint:edit'
/** Komut paletini açan pencere olayı (`CommandPalette` dinler). */
export const PALETTE_EVENT = 'osint:palette'

export interface OsintHandlers {
  root: string | null
  /** Klasörü proje olarak açar (yeni sohbetle). */
  openProject: (root: string) => Promise<void>
  attach: (files: string[]) => void
  send: (text: string) => void
  draft: (text: string) => void
  resume: (id: string) => Promise<void>
  newChat: () => Promise<void>
  showRunning: () => void
  report: (err: unknown) => void
}

async function execute(req: OsintRequest, h: OsintHandlers): Promise<void> {
  for (const step of planRequest(req, h.root)) {
    switch (step.do) {
      case 'confirm':
        if (!(await confirmAction({ title: step.title, message: step.message, confirmLabel: 'Aç' }))) return
        break
      case 'open':
        await h.openProject(step.root)
        break
      case 'attach':
        h.attach(step.files)
        break
      case 'send':
        h.send(step.text)
        break
      case 'draft':
        h.draft(step.text)
        break
      case 'resume':
        await h.resume(step.id)
        break
      case 'edit':
        window.dispatchEvent(new CustomEvent(EDIT_EVENT, { detail: { files: step.files } }))
        break
      case 'palette':
        window.dispatchEvent(new CustomEvent(PALETTE_EVENT))
        break
      case 'newChat':
        await h.newChat()
        break
      case 'running':
        h.showRunning()
        break
    }
  }
}

export function useOsint(handlers: OsintHandlers): void {
  const ref = useRef(handlers)
  ref.current = handlers
  useEffect(() => {
    let stopped = false
    let queue = Promise.resolve()
    // Sırayla: bir onay penceresi açıkken gelen istek beklesin.
    const run = (req: OsintRequest) => {
      queue = queue.then(() => execute(req, ref.current)).catch((err: unknown) => ref.current.report(errorMessage(err)))
    }
    let unlisten: (() => void) | null = null
    // Önce dinleyici kaydı tamamlanır, sonra bekleyenler alınır: arada gelen istek kaybolmaz.
    void listen<OsintRequest>('kor:osint/request', (e) => run(e.payload))
      .then((fn) => {
        if (stopped) fn()
        else unlisten = fn
        return osintApi.pending()
      })
      .then((list) => {
        if (!stopped) list.forEach(run)
      })
      .catch((err: unknown) => ref.current.report(err))
    // Yazma alanı dışına bırakılan tek klasör proje olarak açılır (yazma alanına bırakılanlar ek olur).
    const stopDrop = onNativeDrop((d) => {
      if (d.type !== 'drop' || d.paths.length !== 1) return
      if (document.elementFromPoint(d.x, d.y)?.closest('.composer')) return
      void osintApi
        .drop(d.paths)
        .then((req) => {
          if (req) run(req)
        })
        .catch((err: unknown) => ref.current.report(err))
    })
    return () => {
      stopped = true
      unlisten?.()
      stopDrop()
    }
  }, [])
}

/** App'in hazır hâline takılır (yükleme ekranında değil): istekler ancak işleyiciler varken uygulanır. */
export function OsintHost(props: OsintHandlers): null {
  useOsint(props)
  return null
}
