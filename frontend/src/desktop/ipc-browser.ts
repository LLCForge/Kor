// Ajan tarayıcısı paneli IPC sözleşmesi: YALNIZCA kanal adları + tipler (preload import eder; electron/node yok).
// Handler: handlers/browser-panel.ts. Ajan tarayıcısı sekmeleri ana pencerede WebContentsView olarak çizilir;
// panel yer ayırıp sınırlarını bildirir (web önizlemeyle aynı desen).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { BrowserTabInfo, BrowserBounds, BrowserPanelEvent, BrowserPanelApi } from './ipc.gen.ts'
export type { BrowserTabInfo, BrowserBounds, BrowserPanelEvent, BrowserPanelApi }
export const BROWSER_CHANNELS = {
  show: 'abrowser:show',
  hide: 'abrowser:hide',
  setBounds: 'abrowser:setBounds',
  tabs: 'abrowser:tabs',
  activate: 'abrowser:activate',
  close: 'abrowser:close',
  navigate: 'abrowser:navigate',
  back: 'abrowser:back',
  forward: 'abrowser:forward',
  reload: 'abrowser:reload',
  newTab: 'abrowser:newTab',
  logs: 'abrowser:logs',
  event: 'abrowser:event',
} as const
