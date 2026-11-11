// Maliyet / Bağlam / Bench panelleri (§5.1–5.3, §15.10): `cost.*`, `context.*`, `bench.*`, `packages.cost`.
// Rust: frontend/src-tauri/src/ns/{cost,context,bench,packages}.rs. Alan adları Rust serde çıktısıyla aynıdır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { CostRow, CostReport__cost as CostReport, RouteCategory, RoutingDecision, QuotaKey, QuotaReport, OnLimit, BudgetConfig, RoutingConfig, CostApi, ContextBreakdown__cost as ContextBreakdown, ContextThresholds, ContextLast, ContextApi, BenchArm, BenchOptions, BenchEstimate, BenchRow, BenchSummary, BenchGraphComparison, BenchReport, BenchDone, BenchProgress, BenchApi } from './ipc.gen.ts'
export type { CostRow, CostReport, RouteCategory, RoutingDecision, QuotaKey, QuotaReport, OnLimit, BudgetConfig, RoutingConfig, CostApi, ContextBreakdown, ContextThresholds, ContextLast, ContextApi, BenchArm, BenchOptions, BenchEstimate, BenchRow, BenchSummary, BenchGraphComparison, BenchReport, BenchDone, BenchProgress, BenchApi }

/** `packages.cost` (derived from the generated namespace). */
export type PackagesCostApi = Pick<import('./ipc.gen.ts').PackagesApi, 'cost'>

// ---- context ----

// ---- bench ----
