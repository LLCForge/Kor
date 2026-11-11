// Oturum kaydı/replay ve ortam (§15.9, §15.7; ADR 0043) IPC sözleşmesi.
// Rust: frontend/src-tauri/src/ns/replay.rs (`replay.*`), ns/devenv.rs (`env.status`).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ReplaySummary, ReplayStep, ReplayFile, ReplayEvent, ReplayRecording, ReplayApi, EnvTool, EnvKind, EnvVersion, EnvStatus, EnvApi } from './ipc.gen.ts'
export type { ReplaySummary, ReplayStep, ReplayFile, ReplayEvent, ReplayRecording, ReplayApi, EnvTool, EnvKind, EnvVersion, EnvStatus, EnvApi }
