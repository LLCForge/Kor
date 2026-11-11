// [g31] Uzak bağlantılar (SSH, veritabanı) IPC tipleri. Sırlar arayüze dönmez; yalnızca hasSecret.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ConnectionsView, ConnectionTestResult, SshHostInput, DbConnectionInput, ConnectionsApi } from './ipc.gen.ts'
export type { ConnectionsView, ConnectionTestResult, SshHostInput, DbConnectionInput, ConnectionsApi }

type SecretInput = { password?: string; clearSecret?: boolean }