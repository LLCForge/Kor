// Entegre terminal IPC sözleşmesi: YALNIZCA kanal adları + tipler.
// Preload bu dosyayı import eder (sandbox) → burada electron/node import'u OLMAMALI.
// Handler: handlers/terminal.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { TerminalRunResult, TerminalApi } from './ipc.gen.ts'
export type { TerminalRunResult, TerminalApi }
export const TERMINAL_CHANNELS = { run: 'terminal:run' } as const
