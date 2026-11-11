// Grup 9 (çoklu sağlayıcı ve maliyet) IPC tipleri; ipc.ts'teki g9 işaretlerinden başvurulur. Yalnızca tip.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { KeyProfileView, ProviderView, ProvidersView, G9SettingsPatch, CostInfo, PullState, OllamaView, G9Api } from './ipc.gen.ts'
export type { KeyProfileView, ProviderView, ProvidersView, G9SettingsPatch, CostInfo, PullState, OllamaView, G9Api }
import type { TurnCostInfo } from '../core/agent-g9.ts'
import type { ProbeResult } from '../core/provider-pool-g9.ts'

export type { ProbeResult, TurnCostInfo }
