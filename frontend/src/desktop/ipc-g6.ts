// Grup 6 (bağlam ve token verimliliği) IPC tipleri; ipc.ts'teki g6 işaretlerinden başvurulur. Yalnızca tip: arayüz paketine kod girmez.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ContextView, CompactSettings, CompactSettingsPatch, ProjectMapInfo } from './ipc.gen.ts'
export type { ContextView, CompactSettings, CompactSettingsPatch, ProjectMapInfo }
import type { ContextInfo } from '../core/context.ts'
import type { MapProgress } from '../core/project-map-build.ts'
import type { ProjectMapStatus } from '../core/project-map.ts'

export type { CompactionEvent, CompactionResult } from '../core/compaction.ts'
export type { ContextInfo } from '../core/context.ts'
export type { InitProjectResult, MapProgress } from '../core/project-map-build.ts'
export type { ProjectMapStatus } from '../core/project-map.ts'
