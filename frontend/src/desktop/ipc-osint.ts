// İşletim sistemi entegrasyonu (§25.1, §25.3, §27.6; ADR 0052). Rust: frontend/src-tauri/src/ns/osint.rs, osint.rs.
// İstekler `kor_runtime::osint::Request` (serde `kind` etiketi) ile aynı biçimdedir.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { OsintOrigin, QuickTask, OsintRequest, HotkeyStatus, MenuStatus, OsintApi } from './ipc.gen.ts'
export type { OsintOrigin, QuickTask, OsintRequest, HotkeyStatus, MenuStatus, OsintApi }
