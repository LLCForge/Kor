// Görev şablonları (§15.11): `kor templates list|show|new` ve `kor run template:<ad>` karşılıkları.
// Rust: frontend/src-tauri/src/ns/templates.rs. Doğrulama ve işleme çekirdekte (kor_compat::templates).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { TemplateInputKind, TemplateInput, TaskTemplate, RenderedTemplate, TemplatesApi } from './ipc.gen.ts'
export type { TemplateInputKind, TemplateInput, TaskTemplate, RenderedTemplate, TemplatesApi }
