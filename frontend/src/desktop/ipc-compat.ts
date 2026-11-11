// Uyumluluk paneli IPC sözleşmesi (§3.3). Rust: frontend/src-tauri/src/ns/compat.rs. Genel durum
// `core.compat` (ipc-core.ts CompatStatus); burada çakışmalar, MCP, dışa aktarma ve kaynağı gösterme.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { CompatSource, CompatApplies, CompatConflict, CompatExportTarget, CompatApi } from './ipc.gen.ts'
export type { CompatSource, CompatApplies, CompatConflict, CompatExportTarget, CompatApi }
