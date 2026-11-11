// Çok ajanlı görevler (§8.2, ADR 0037): `tasks.*`. Rust: frontend/src-tauri/src/ns/tasks.rs,
// alan adları `kor_runtime::task::TaskState` serde çıktısıyla aynıdır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { TaskStatus, SubStatus, TaskTestRun, SubTask, ConflictFile, TaskConflict, TaskSettings, TaskState, TaskStartOptions, TasksApi } from './ipc.gen.ts'
export type { TaskStatus, SubStatus, TaskTestRun, SubTask, ConflictFile, TaskConflict, TaskSettings, TaskState, TaskStartOptions, TasksApi }
