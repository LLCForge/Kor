// Günlükler paneli IPC sözleşmesi (cdesktop "Logs"): YALNIZCA kanal adları + tipler.
// Preload bu dosyayı import eder (sandbox) → electron/node import'u OLMAMALI. Handler: handlers/logs.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { LogEntry, LogsApi } from './ipc.gen.ts'
export type { LogEntry, LogsApi }
export const LOGS_CHANNELS = { list: 'logs:list' } as const
