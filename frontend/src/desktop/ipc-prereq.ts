// Sistem gereksinimleri, Katman C (§24.3, ADR 0064): `prereq.*`. Rust: frontend/src-tauri/src/ns/prereq.rs.
// Tipler IPC şemasından üretilir (backend/crates/kor-core/schema/ipc.json → ipc.gen.ts, ADR 0061).
export type { PrereqApi, PrereqId, PrereqInstallResult, PrereqProgress, PrereqRow } from './ipc.gen.ts'
