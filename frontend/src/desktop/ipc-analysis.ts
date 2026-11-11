// Analiz paneli (§9 `kor audit --security|--compliance`, ADR 0045). Rust: frontend/src-tauri/src/ns/analysis.rs.
// ipc.ts bu ad alanını tiplemiyor; erişim renderer/panels/analysis/bridge.ts içinden.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { Severity, Finding, AnalysisReport, ExportFormat__analysis as ExportFormat, ExportResult, VerifyResult, AnalysisApi } from './ipc.gen.ts'
export type { Severity, Finding, AnalysisReport, ExportFormat, ExportResult, VerifyResult, AnalysisApi }
