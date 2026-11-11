// Routines (zamanlanmış görevler) IPC sözleşmesi: YALNIZCA kanal adları + tipler.
// Bu dosyayı preload import eder; preload sandbox'ta çalışır, bu yüzden burada electron/node
// import'u OLMAMALI (handler'lar handlers/routines.ts'te). g4 deseniyle aynı ayrım.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { RoutineInput, RoutinesApi } from './ipc.gen.ts'
export type { RoutineInput, RoutinesApi }

export const ROUTINE_CHANNELS = {
  list: 'routines:list',
  add: 'routines:add',
  remove: 'routines:remove',
  setEnabled: 'routines:setEnabled',
} as const
