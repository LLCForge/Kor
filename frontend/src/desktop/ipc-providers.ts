// Sağlayıcılar paneli (G4) köprü tipleri: `provider.*`, `catalog.*`, `proxy.*`, `components.*`, `media.*`.
// Rust tarafı `src-tauri/src/ns/{provider,catalog,proxy,components,media}.rs`. Yalnızca tip.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ProviderErrorView, ProviderCheck, ProviderTestResult, ProviderStatusRow, ProviderApi, CatalogEntry__providers as CatalogEntry, CatalogResult, CatalogApi, ProxyModelUsage, ProxyCredential, ProxyStatus, ProxyStart, ProxyConfig, ProxyApi, ComponentView, ComponentsApi, RuntimeTick, MediaJob, HfModel, HfInfo, MediaProgress, HfProgress, MediaApi } from './ipc.gen.ts'
export type { ProviderErrorView, ProviderCheck, ProviderTestResult, ProviderStatusRow, ProviderApi, CatalogEntry, CatalogResult, CatalogApi, ProxyModelUsage, ProxyCredential, ProxyStatus, ProxyStart, ProxyConfig, ProxyApi, ComponentView, ComponentsApi, RuntimeTick, MediaJob, HfModel, HfInfo, MediaProgress, HfProgress, MediaApi }

export type ProvidersPanelApi = Pick<import('./ipc.gen.ts').KorIpcApi, 'provider' | 'catalog' | 'proxy' | 'components' | 'media' | 'onRuntimeProgress'>
