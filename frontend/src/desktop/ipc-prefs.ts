// ADR 0050 IPC sözleşmesi: otomasyonlar (§28.14), okunabilirlik (§28.9), tuş düzeni (§28.10),
// açıklayıcı soru eşiği (§28.13), planlar (§28.2), oturum ağacı (§28.6), `kor doctor --fix` (§26.1).
// Rust: frontend/src-tauri/src/ns/prefs.rs (`prefs.*`, `plan.*`, `sessions.*`).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { AutoRow, LineWidth, Density, DiffViewMode, ToolOutput, KeyMode, UiPrefs, AskThreshold, AgentPrefs, Keymap, DoctorFix, PrefsApi, QuestionAnswer, CompactAnswer, QuestionEvent, CompactItem, CompactPreviewEvent, PlanInfo, PlanApi, SessionEntry, SessionHit, SessionFilter, SessionColor, SessionsApi } from './ipc.gen.ts'
export type { AutoRow, LineWidth, Density, DiffViewMode, ToolOutput, KeyMode, UiPrefs, AskThreshold, AgentPrefs, Keymap, DoctorFix, PrefsApi, QuestionAnswer, CompactAnswer, QuestionEvent, CompactItem, CompactPreviewEvent, PlanInfo, PlanApi, SessionEntry, SessionHit, SessionFilter, SessionColor, SessionsApi }

export const SESSION_COLORS = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'gray'] as const