// Güvenlik paneli (§6.4 sandbox, §6.6 kasa, §6.7 denetim kaydı). Rust: frontend/src-tauri/src/ns/security.rs.
// ipc.ts bu ad alanlarını tiplemiyor; erişim renderer/panels/security/bridge.ts içinden.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { VaultList, VaultApi, VaultImport, AuditQuery, AuditEntry, AuditPage, AuditHead, AuditVerify, AuditReport, AuditApi, SandboxMode, SandboxPreference, SandboxStatus__security as SandboxStatus, SandboxProbe, SandboxApi } from './ipc.gen.ts'
export type { VaultList, VaultApi, VaultImport, AuditQuery, AuditEntry, AuditPage, AuditHead, AuditVerify, AuditReport, AuditApi, SandboxMode, SandboxPreference, SandboxStatus, SandboxProbe, SandboxApi }
