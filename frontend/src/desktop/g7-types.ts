// Grup 7 IPC tipleri (ipc.ts'te import() tipleriyle kullanılır).
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ThemeSetting, NotificationSound, NotificationPrefs, GitPrefs, ScriptPrefs, AgentProfile, EditorType, EditorPrefs, CatalogResponse, G7Settings, PermissionRulesInfo, TrustInfo, PrivacyFolder, PrivacyClearTarget, PrivacyInfo } from './ipc.gen.ts'
export type { ThemeSetting, NotificationSound, NotificationPrefs, GitPrefs, ScriptPrefs, AgentProfile, EditorType, EditorPrefs, CatalogResponse, G7Settings, PermissionRulesInfo, TrustInfo, PrivacyFolder, PrivacyClearTarget, PrivacyInfo }
import type { CatalogModel, TaskModels } from '../core/model-catalog.ts'
import type { ProbeResult } from '../core/model-probe.ts'
import type { UsageHistory, UsagePeriod } from '../core/usage-history.ts'

export type { CatalogModel, ProbeResult, TaskModels, UsageHistory, UsagePeriod }
