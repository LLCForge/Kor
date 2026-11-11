// [g12] Güvenlik paneli (sandbox, oturum araç izinleri) ve toplu silme onayı için IPC tipleri.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { G12ToolLevel, G12ToolScope } from './ipc.gen.ts'
export type { G12ToolLevel, G12ToolScope }
import type { BulkDeleteRequest } from '../core/bulk-delete-g12.ts'
import type { SandboxStatus } from '../core/sandbox-g12.ts'

export type { BulkDeleteRequest, SandboxStatus }

/** default: izin moduna göre; ask: her seferinde sor; allow: bu sohbette izin ver; deny: bu sohbette kapalı */
export const G12_TOOL_LEVELS = ['default', 'ask', 'allow', 'deny'] as const