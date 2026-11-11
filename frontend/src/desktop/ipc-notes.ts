// Notlar paneli IPC sözleşmesi (cdesktop "Notes"): YALNIZCA kanal adları + tipler.
// Preload bu dosyayı import eder (sandbox) → electron/node import'u OLMAMALI. Handler: handlers/notes.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { NotesApi } from './ipc.gen.ts'
export type { NotesApi }
export const NOTES_CHANNELS = { get: 'notes:get', set: 'notes:set' } as const
