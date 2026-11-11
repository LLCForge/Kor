// Grup 16 (dağıtım ve tanılama) IPC tipleri; ipc.ts'teki g16 işaretlerinden başvurulur. Yalnızca tip: arayüz paketine kod girmez.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { Distribution, UpdatePhase, UpdateStatus, CliStatus, CrashInfo, ConfigImportResult } from './ipc.gen.ts'
export type { Distribution, UpdatePhase, UpdateStatus, CliStatus, CrashInfo, ConfigImportResult }
import type { CrashEvent, DumpInfo } from '../core/g16-crash.ts'
import type { ImportSummary } from '../core/g16-config.ts'

export type { CrashEvent, DumpInfo, ImportSummary }

/** ipc.ts'te tiplenmeyen g16 yöntemleri: `api as unknown as G16PackagingApi` */
export type G16PackagingApi = Pick<import('./ipc.gen.ts').KorIpcApi, 'g16CliStatus' | 'g16CliInstall' | 'g16CliRemove'> & { g16: Pick<import('./ipc.gen.ts').G16Api, 'onUpdateAvailable'> }
