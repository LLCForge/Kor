// Hafıza paneli IPC sözleşmesi (§6.1, §31.3). Rust: frontend/src-tauri/src/ns/memory.rs (`memory.*`)
// ve ns/projmem.rs (`projmem.*`). ipc.ts bu ad alanlarını tiplemez; panel tipli bir dönüşümle erişir.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { KnowledgeRecord, RecallHit, Decision, MemorySection, MemoryDocFile, MemoryOverview__memory as MemoryOverview, MemoryApi, ProjMemApi } from './ipc.gen.ts'
export type { KnowledgeRecord, RecallHit, Decision, MemorySection, MemoryDocFile, MemoryOverview, MemoryApi, ProjMemApi }
