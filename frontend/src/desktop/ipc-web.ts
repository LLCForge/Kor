// Web siteleri (§29, ADR 0054). Rust: frontend/src-tauri/src/ns/web.rs. ipc.ts bu ad alanını tiplemiyor;
// erişim renderer/panels/web/bridge.ts içinden. Her yöntem tek bir nesne alır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { WebArgs, PlanRow, VerifyResult__web as VerifyResult, RunReport, HostItem, SiteItem, ClientItem, WebApi } from './ipc.gen.ts'
export type { WebArgs, PlanRow, VerifyResult, RunReport, HostItem, SiteItem, ClientItem, WebApi }
