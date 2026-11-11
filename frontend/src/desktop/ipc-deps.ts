// Bağımlılıklar (§9 `kor deps list|check`, `kor audit --licenses`, ADR 0046). Rust:
// frontend/src-tauri/src/ns/deps.rs. ipc.ts bu ad alanını tiplemiyor; erişim
// renderer/panels/analysis/DepsSection.tsx içinden.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { LicenseClass, DepPackage, DepsList, DepsApi } from './ipc.gen.ts'
export type { LicenseClass, DepPackage, DepsList, DepsApi }
