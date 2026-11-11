// Dağıtım ve log izleme (§8.6, §8.7, ADR 0041): `deploy.*`, `watch.*` ve §8.5 `db.*` ekleri.
// Rust: frontend/src-tauri/src/ns/deploy.rs, ns/watch.rs, ns/db.rs; alan adları
// `kor_runtime::deploy`, `kor_runtime::watch`, `kor_runtime::dbops` serde çıktısıyla aynıdır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { StepKind__deploy as StepKind, DeployStep, DeployPlan, DeployEvent, HealthResult, DeployReport, EnvState, DeployEnvs, Risk__deploy as Risk, RiskReport, DeployApi, Fix, Incident, WatchEvent, Watcher, WatchApi, IndexSuggestion, MigrationStyle, DbOpsApi } from './ipc.gen.ts'
export type { StepKind, DeployStep, DeployPlan, DeployEvent, HealthResult, DeployReport, EnvState, DeployEnvs, Risk, RiskReport, DeployApi, Fix, Incident, WatchEvent, Watcher, WatchApi, IndexSuggestion, MigrationStyle, DbOpsApi }
