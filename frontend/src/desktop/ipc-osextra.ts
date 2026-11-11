// İşletim sistemine özgü entegrasyonlar (§27.10, ADR 0052). Rust: frontend/src-tauri/src/ns/osextra.rs,
// iş mantığı kor_platform::os_integration (CLI: `kor os-integration`).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { OsName, OsExtraKind, OsExtraStatus, OsExtraList, PortalOutcome, OsExtraApi } from './ipc.gen.ts'
export type { OsName, OsExtraKind, OsExtraStatus, OsExtraList, PortalOutcome, OsExtraApi }
