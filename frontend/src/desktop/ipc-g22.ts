// [g22] Evrensel model desteği IPC tipleri: ipc.ts'teki KorApi bu tiplere satır içi import ile başvurur.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { G22FileSupport, G22ModelSupport } from './ipc.gen.ts'
export type { G22FileSupport, G22ModelSupport }
import type { ToolMode } from '../core/text-tools-g22.ts'

export type { ToolMode }
