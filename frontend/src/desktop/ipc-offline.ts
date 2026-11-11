// Çevrimdışı / air-gap (§6.5, ADR 0034): `offline.*`. Rust: frontend/src-tauri/src/ns/offline.rs,
// alan adları `kor_runtime::offline` serde çıktısıyla aynıdır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { OfflineServer, OfflineModel, OfflineMissing, OfflineStatus, OfflineInitOptions, OfflineDownload, OfflinePlan, OfflineStep, OfflineEvent, OfflineDone, OfflineBundleOptions, OfflineApi } from './ipc.gen.ts'
export type { OfflineServer, OfflineModel, OfflineMissing, OfflineStatus, OfflineInitOptions, OfflineDownload, OfflinePlan, OfflineStep, OfflineEvent, OfflineDone, OfflineBundleOptions, OfflineApi }
