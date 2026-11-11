// Tetikleyiciler ve kor daemon (§8.4, §11.3, ADR 0039): `triggers.*`. Rust: frontend/src-tauri/src/ns/triggers.rs,
// alan adları `kor_runtime::triggers` serde çıktısıyla aynıdır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { TriggerEvent, TriggerApproval, TriggerRow, TriggerInput, JobStatus, TriggerJob, WaitingApproval, DaemonStatus, NotifySettings, NotifyTest, TriggersApi } from './ipc.gen.ts'
export type { TriggerEvent, TriggerApproval, TriggerRow, TriggerInput, JobStatus, TriggerJob, WaitingApproval, DaemonStatus, NotifySettings, NotifyTest, TriggersApi }
