// Niyet katmanı (§30) ve araç kataloğu (§26.2) IPC sözleşmesi, ADR 0056.
// Rust: frontend/src-tauri/src/ns/intent.rs (`intent.*`) ve ns/tools.rs (`tools.*`);
// iş mantığı kor_runtime::intent ve kor_agent::clitools.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { IntentRun, IntentVia, IntentPlan, IntentStep, IntentDecision, IntentRouteContext, IntentRunResult, IntentPhrase, IntentEntry, IntentPattern, IntentApi, ToolEntry, TypedTool, ToolHelp, ToolsApi } from './ipc.gen.ts'
export type { IntentRun, IntentVia, IntentPlan, IntentStep, IntentDecision, IntentRouteContext, IntentRunResult, IntentPhrase, IntentEntry, IntentPattern, IntentApi, ToolEntry, TypedTool, ToolHelp, ToolsApi }
