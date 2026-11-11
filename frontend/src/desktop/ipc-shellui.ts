// Kabuk varlığı (§27.1 tepsi, §27.2 bildirim merkezi, §27.4 güncelleme pill'i, §27.8 PiP, §27.9 görev çubuğu,
// §28.4 "beni çağır", §28.5 ETA ve onay kuyruğu; ADR 0051): `shellui.*`. Rust: frontend/src-tauri/src/ns/shellui.rs,
// alan adları `kor_runtime::presence` serde çıktısıyla aynıdır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { NotificationCategory, KorNotification, Alert, EtaScope, Eta, RunningSession, ApprovalSource, Risk, QueueItem, QueueGroup, DaemonJob, RemoteTask, Overview__shellui as Overview, ShellState, ShellSettings, ShellSettingKey, ShellUiApi } from './ipc.gen.ts'
export type { NotificationCategory, KorNotification, Alert, EtaScope, Eta, RunningSession, ApprovalSource, Risk, QueueItem, QueueGroup, DaemonJob, RemoteTask, Overview, ShellState, ShellSettings, ShellSettingKey, ShellUiApi }
