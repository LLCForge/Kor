// `kor perf` (§15.8, ADR 0049) IPC sözleşmesi. Rust: frontend/src-tauri/src/ns/perf.rs (`perf.*`),
// iş mantığı kor_runtime::perf. Tüm sayılar ölçümdür (profiler örnekleri, Kor'un saati, yük üreteci).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { PerfUnit, PerfNodeLink, PerfRow, PerfLoadStats, PerfCompareRow, PerfReport, PerfSummary, PerfRunOptions, PerfApi } from './ipc.gen.ts'
export type { PerfUnit, PerfNodeLink, PerfRow, PerfLoadStats, PerfCompareRow, PerfReport, PerfSummary, PerfRunOptions, PerfApi }
