import type { Text } from '@codemirror/state'

// Bileşen ağacının farklı yerlerindeki grup 4 parçalarının paylaştığı küçük durum.
// Ortak dosyalara (Sidebar, App) yeni prop zinciri eklememek için modül düzeyinde tutulur.

// ---------- Sohbete ekleme isteği (dosya ağacı → App) ----------

type AttachListener = (path: string) => void
const attachListeners = new Set<AttachListener>()

export function onAttachRequest(listener: AttachListener): () => void {
  attachListeners.add(listener)
  return () => {
    attachListeners.delete(listener)
  }
}

export function requestAttach(path: string): void {
  for (const listener of attachListeners) listener(path)
}

// ---------- Editör taslakları ----------
// Önizleme paneli dosya değişince yeniden kurulur; kaydedilmemiş metin kaybolmasın diye taslaklar burada yaşar.

export interface Draft {
  doc: Text
  /** Taslağın dayandığı disk sürümü (çakışma denetimi) */
  baseMtimeMs: number
}

const drafts = new Map<string, Draft>()

// Pencere kapanırken taslak varsa ana süreç (will-prevent-unload) kullanıcıya sorar.
const blockUnload = (e: BeforeUnloadEvent) => e.preventDefault()

function syncUnloadGuard(): void {
  if (drafts.size > 0) window.addEventListener('beforeunload', blockUnload)
  else window.removeEventListener('beforeunload', blockUnload)
}

export function getDraft(path: string): Draft | undefined {
  return drafts.get(path)
}

export function setDraft(path: string, draft: Draft): void {
  drafts.set(path, draft)
  syncUnloadGuard()
}

export function clearDraft(path: string): void {
  drafts.delete(path)
  syncUnloadGuard()
}

export function draftPaths(): string[] {
  return [...drafts.keys()]
}

export function clearAllDrafts(): void {
  drafts.clear()
  syncUnloadGuard()
}
