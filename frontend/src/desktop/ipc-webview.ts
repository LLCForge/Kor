// `webview.*` köprüsü ve pencere olayları (spec §21, ADR 0053). Rust: src-tauri/src/ns/webview.rs, webview_lock.rs.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { WebviewApi, WindowEvents } from './ipc.gen.ts'
export type { WebviewApi, WindowEvents }
import { api } from './renderer/api.ts'

export const webviewApi = (api as unknown as { webview: WebviewApi }).webview
export const windowEvents = (api as unknown as { window: WindowEvents }).window
