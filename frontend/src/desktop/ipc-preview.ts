// Web önizleme (cdesktop Dev Server Preview + Browser Companion) IPC sözleşmesi: YALNIZCA kanal adları + tipler.
// Preload bu dosyayı import eder (sandbox) → electron/node import'u OLMAMALI. Handler: handlers/preview.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { DevServerStatus, PreviewBounds, PreviewSelection, PreviewEvent, PreviewApi } from './ipc.gen.ts'
export type { DevServerStatus, PreviewBounds, PreviewSelection, PreviewEvent, PreviewApi }
export const PREVIEW_CHANNELS = {
  devStart: 'preview:dev:start',
  devStop: 'preview:dev:stop',
  devStatus: 'preview:dev:status',
  show: 'preview:show',
  hide: 'preview:hide',
  setBounds: 'preview:setBounds',
  navigate: 'preview:navigate',
  reload: 'preview:reload',
  back: 'preview:back',
  setInspect: 'preview:setInspect',
  openExternal: 'preview:openExternal',
  event: 'preview:event',
  /** Companion preload (önizleme sayfası) → main: seçilen öğe */
  companionPick: 'preview:companion:pick',
  /** main → companion preload: incele modunu aç/kapat */
  companionInspect: 'preview:companion:inspect',
} as const
