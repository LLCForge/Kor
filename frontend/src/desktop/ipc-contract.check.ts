// Compile-time IPC contract check (spec §16.3, ADR 0061): every handwritten IPC type that still exists —
// ipc.ts (KorApi, owned by the orchestrator) and the core/*.ts domain types the ipc-*.ts files re-export —
// must equal its generated counterpart in ipc.gen.ts. `npm run typecheck` fails on drift.
import type * as G from './ipc.gen.ts'
type Eq<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false
function ok<T extends true>(): void {}
/** ipc.ts lags the Rust bridge (no `status` item, `confirm` without review fields): until the orchestrator swaps it
 *  for the generated types (ADR 0061 snippet) it may describe less than the schema, never something else. */
type Sub<A, B> = [A] extends [B] ? true : false
import type * as H0 from './ipc-agents.ts'
ok<Eq<H0.AgentSummary, G.AgentSummary>>() // ipc-agents.ts AgentSummary
ok<Eq<H0.AgentsApi, G.AgentsApi>>() // ipc-agents.ts AgentsApi
import type * as H1 from './ipc-analysis.ts'
ok<Eq<H1.Severity, G.Severity>>() // ipc-analysis.ts Severity
ok<Eq<H1.Finding, G.Finding>>() // ipc-analysis.ts Finding
ok<Eq<H1.AnalysisReport, G.AnalysisReport>>() // ipc-analysis.ts AnalysisReport
ok<Eq<H1.ExportFormat, G.ExportFormat__analysis>>() // ipc-analysis.ts ExportFormat
ok<Eq<H1.ExportResult, G.ExportResult>>() // ipc-analysis.ts ExportResult
ok<Eq<H1.VerifyResult, G.VerifyResult>>() // ipc-analysis.ts VerifyResult
ok<Eq<H1.AnalysisApi, G.AnalysisApi>>() // ipc-analysis.ts AnalysisApi
import type * as H2 from './ipc-batch.ts'
ok<Eq<H2.BatchOp, G.BatchOp>>() // ipc-batch.ts BatchOp
ok<Eq<H2.BatchRow, G.BatchRow>>() // ipc-batch.ts BatchRow
ok<Eq<H2.BatchPlan, G.BatchPlan>>() // ipc-batch.ts BatchPlan
ok<Eq<H2.BatchApplied, G.BatchApplied>>() // ipc-batch.ts BatchApplied
ok<Eq<H2.BatchApi, G.BatchApi>>() // ipc-batch.ts BatchApi
import type * as H3 from './ipc-browser.ts'
ok<Eq<H3.BrowserTabInfo, G.BrowserTabInfo>>() // ipc-browser.ts BrowserTabInfo
ok<Eq<H3.BrowserBounds, G.BrowserBounds>>() // ipc-browser.ts BrowserBounds
ok<Eq<H3.BrowserPanelEvent, G.BrowserPanelEvent>>() // ipc-browser.ts BrowserPanelEvent
ok<Eq<H3.BrowserPanelApi, G.BrowserPanelApi>>() // ipc-browser.ts BrowserPanelApi
import type * as H4 from './ipc-chrome.ts'
ok<Eq<H4.ChromeBridgeInfo, G.ChromeBridgeInfo>>() // ipc-chrome.ts ChromeBridgeInfo
ok<Eq<H4.ChromeExtensionApi, G.ChromeExtensionApi>>() // ipc-chrome.ts ChromeExtensionApi
import type * as H5 from './ipc-commands.ts'
ok<Eq<H5.CommandsApi, G.CommandsApi>>() // ipc-commands.ts CommandsApi
import type * as H6 from './ipc-compat.ts'
ok<Eq<H6.CompatSource, G.CompatSource>>() // ipc-compat.ts CompatSource
ok<Eq<H6.CompatApplies, G.CompatApplies>>() // ipc-compat.ts CompatApplies
ok<Eq<H6.CompatConflict, G.CompatConflict>>() // ipc-compat.ts CompatConflict
ok<Eq<H6.CompatExportTarget, G.CompatExportTarget>>() // ipc-compat.ts CompatExportTarget
ok<Eq<H6.CompatApi, G.CompatApi>>() // ipc-compat.ts CompatApi
import type * as H7 from './ipc-connections.ts'
ok<Eq<H7.ConnectionsView, G.ConnectionsView>>() // ipc-connections.ts ConnectionsView
ok<Eq<H7.ConnectionTestResult, G.ConnectionTestResult>>() // ipc-connections.ts ConnectionTestResult
ok<Eq<H7.SshHostInput, G.SshHostInput>>() // ipc-connections.ts SshHostInput
ok<Eq<H7.DbConnectionInput, G.DbConnectionInput>>() // ipc-connections.ts DbConnectionInput
ok<Eq<H7.ConnectionsApi, G.ConnectionsApi>>() // ipc-connections.ts ConnectionsApi
import type * as H8 from './ipc-core.ts'
ok<Eq<H8.CoreSource, G.CoreSource>>() // ipc-core.ts CoreSource
ok<Eq<H8.CompatStatus, G.CompatStatus>>() // ipc-core.ts CompatStatus
ok<Eq<H8.ConfigExplain, G.ConfigExplain>>() // ipc-core.ts ConfigExplain
ok<Eq<H8.UndoJournal, G.UndoJournal>>() // ipc-core.ts UndoJournal
ok<Eq<H8.UndoReport, G.UndoReport>>() // ipc-core.ts UndoReport
ok<Eq<H8.Marketplace, G.Marketplace>>() // ipc-core.ts Marketplace
ok<Eq<H8.CoreApi, G.CoreApi>>() // ipc-core.ts CoreApi
import type * as H9 from './ipc-cost.ts'
ok<Eq<H9.CostRow, G.CostRow>>() // ipc-cost.ts CostRow
ok<Eq<H9.CostReport, G.CostReport__cost>>() // ipc-cost.ts CostReport
ok<Eq<H9.RouteCategory, G.RouteCategory>>() // ipc-cost.ts RouteCategory
ok<Eq<H9.RoutingDecision, G.RoutingDecision>>() // ipc-cost.ts RoutingDecision
ok<Eq<H9.QuotaKey, G.QuotaKey>>() // ipc-cost.ts QuotaKey
ok<Eq<H9.QuotaReport, G.QuotaReport>>() // ipc-cost.ts QuotaReport
ok<Eq<H9.OnLimit, G.OnLimit>>() // ipc-cost.ts OnLimit
ok<Eq<H9.BudgetConfig, G.BudgetConfig>>() // ipc-cost.ts BudgetConfig
ok<Eq<H9.RoutingConfig, G.RoutingConfig>>() // ipc-cost.ts RoutingConfig
ok<Eq<H9.CostApi, G.CostApi>>() // ipc-cost.ts CostApi
ok<Eq<H9.ContextBreakdown, G.ContextBreakdown__cost>>() // ipc-cost.ts ContextBreakdown
ok<Eq<H9.ContextThresholds, G.ContextThresholds>>() // ipc-cost.ts ContextThresholds
ok<Eq<H9.ContextLast, G.ContextLast>>() // ipc-cost.ts ContextLast
ok<Eq<H9.ContextApi, G.ContextApi>>() // ipc-cost.ts ContextApi
ok<Eq<H9.BenchArm, G.BenchArm>>() // ipc-cost.ts BenchArm
ok<Eq<H9.BenchOptions, G.BenchOptions>>() // ipc-cost.ts BenchOptions
ok<Eq<H9.BenchEstimate, G.BenchEstimate>>() // ipc-cost.ts BenchEstimate
ok<Eq<H9.BenchRow, G.BenchRow>>() // ipc-cost.ts BenchRow
ok<Eq<H9.BenchSummary, G.BenchSummary>>() // ipc-cost.ts BenchSummary
ok<Eq<H9.BenchGraphComparison, G.BenchGraphComparison>>() // ipc-cost.ts BenchGraphComparison
ok<Eq<H9.BenchReport, G.BenchReport>>() // ipc-cost.ts BenchReport
ok<Eq<H9.BenchDone, G.BenchDone>>() // ipc-cost.ts BenchDone
ok<Eq<H9.BenchProgress, G.BenchProgress>>() // ipc-cost.ts BenchProgress
ok<Eq<H9.BenchApi, G.BenchApi>>() // ipc-cost.ts BenchApi
import type * as H10 from './ipc-deploy.ts'
ok<Eq<H10.StepKind, G.StepKind__deploy>>() // ipc-deploy.ts StepKind
ok<Eq<H10.DeployStep, G.DeployStep>>() // ipc-deploy.ts DeployStep
ok<Eq<H10.DeployPlan, G.DeployPlan>>() // ipc-deploy.ts DeployPlan
ok<Eq<H10.DeployEvent, G.DeployEvent>>() // ipc-deploy.ts DeployEvent
ok<Eq<H10.HealthResult, G.HealthResult>>() // ipc-deploy.ts HealthResult
ok<Eq<H10.DeployReport, G.DeployReport>>() // ipc-deploy.ts DeployReport
ok<Eq<H10.EnvState, G.EnvState>>() // ipc-deploy.ts EnvState
ok<Eq<H10.DeployEnvs, G.DeployEnvs>>() // ipc-deploy.ts DeployEnvs
ok<Eq<H10.Risk, G.Risk__deploy>>() // ipc-deploy.ts Risk
ok<Eq<H10.RiskReport, G.RiskReport>>() // ipc-deploy.ts RiskReport
ok<Eq<H10.DeployApi, G.DeployApi>>() // ipc-deploy.ts DeployApi
ok<Eq<H10.Fix, G.Fix>>() // ipc-deploy.ts Fix
ok<Eq<H10.Incident, G.Incident>>() // ipc-deploy.ts Incident
ok<Eq<H10.WatchEvent, G.WatchEvent>>() // ipc-deploy.ts WatchEvent
ok<Eq<H10.Watcher, G.Watcher>>() // ipc-deploy.ts Watcher
ok<Eq<H10.WatchApi, G.WatchApi>>() // ipc-deploy.ts WatchApi
ok<Eq<H10.IndexSuggestion, G.IndexSuggestion>>() // ipc-deploy.ts IndexSuggestion
ok<Eq<H10.MigrationStyle, G.MigrationStyle>>() // ipc-deploy.ts MigrationStyle
ok<Eq<H10.DbOpsApi, G.DbOpsApi>>() // ipc-deploy.ts DbOpsApi
import type * as H11 from './ipc-deps.ts'
ok<Eq<H11.LicenseClass, G.LicenseClass>>() // ipc-deps.ts LicenseClass
ok<Eq<H11.DepPackage, G.DepPackage>>() // ipc-deps.ts DepPackage
ok<Eq<H11.DepsList, G.DepsList>>() // ipc-deps.ts DepsList
ok<Eq<H11.DepsApi, G.DepsApi>>() // ipc-deps.ts DepsApi
import type * as H12 from './ipc-find.ts'
ok<Eq<H12.FindSource, G.FindSource>>() // ipc-find.ts FindSource
ok<Eq<H12.FindHit, G.FindHit>>() // ipc-find.ts FindHit
ok<Eq<H12.FindLink, G.FindLink>>() // ipc-find.ts FindLink
ok<Eq<H12.Found, G.Found>>() // ipc-find.ts Found
ok<Eq<H12.FindResult, G.FindResult>>() // ipc-find.ts FindResult
ok<Eq<H12.FindApi, G.FindApi>>() // ipc-find.ts FindApi
import type * as H13 from './ipc-forge.ts'
ok<Eq<H13.ForgeKind, G.ForgeKind>>() // ipc-forge.ts ForgeKind
ok<Eq<H13.ForgeRemote, G.ForgeRemote>>() // ipc-forge.ts ForgeRemote
ok<Eq<H13.ForgeDetect, G.ForgeDetect>>() // ipc-forge.ts ForgeDetect
ok<Eq<H13.PrInfo, G.PrInfo>>() // ipc-forge.ts PrInfo
ok<Eq<H13.PrRef, G.PrRef>>() // ipc-forge.ts PrRef
ok<Eq<H13.PrDescription, G.PrDescription>>() // ipc-forge.ts PrDescription
ok<Eq<H13.Severity, G.Severity__forge>>() // ipc-forge.ts Severity
ok<Eq<H13.Finding, G.Finding__forge>>() // ipc-forge.ts Finding
ok<Eq<H13.ReviewReport, G.ReviewReport>>() // ipc-forge.ts ReviewReport
ok<Eq<H13.ReviewResult, G.ReviewResult>>() // ipc-forge.ts ReviewResult
ok<Eq<H13.PersonaInfo, G.PersonaInfo>>() // ipc-forge.ts PersonaInfo
ok<Eq<H13.PrComment, G.PrComment>>() // ipc-forge.ts PrComment
ok<Eq<H13.RespondDraft, G.RespondDraft>>() // ipc-forge.ts RespondDraft
ok<Eq<H13.ReviewOptions, G.ReviewOptions>>() // ipc-forge.ts ReviewOptions
ok<Eq<H13.ForgeApi, G.ForgeApi>>() // ipc-forge.ts ForgeApi
import type * as H14 from './ipc-g10.ts'
ok<Eq<H14.RecoverableTurn, G.RecoverableTurn>>() // ipc-g10.ts RecoverableTurn
ok<Eq<H14.RestoreResult, G.RestoreResult>>() // ipc-g10.ts RestoreResult
ok<Eq<H14.G10Api, G.G10Api>>() // ipc-g10.ts G10Api
ok<Eq<H14.CheckItem, G.CheckItem>>() // ipc-g10.ts CheckItem
ok<Eq<H14.CheckStatus, G.CheckStatus>>() // ipc-g10.ts CheckStatus
ok<Eq<H14.DeadLetter, G.DeadLetter>>() // ipc-g10.ts DeadLetter
ok<Eq<H14.G10Event, G.G10Event>>() // ipc-g10.ts G10Event
import type * as H15 from './ipc-g12.ts'
ok<Eq<H15.G12ToolLevel, G.G12ToolLevel>>() // ipc-g12.ts G12ToolLevel
ok<Eq<H15.G12ToolScope, G.G12ToolScope>>() // ipc-g12.ts G12ToolScope
ok<Eq<H15.BulkDeleteRequest, G.BulkDeleteRequest>>() // ipc-g12.ts BulkDeleteRequest
ok<Eq<H15.SandboxStatus, G.SandboxStatus>>() // ipc-g12.ts SandboxStatus
import type * as H16 from './ipc-g13.ts'
ok<Eq<H16.PromptLayers, G.PromptLayers>>() // ipc-g13.ts PromptLayers
ok<Eq<H16.EnvKey, G.EnvKey>>() // ipc-g13.ts EnvKey
ok<Eq<H16.ExportFormat, G.ExportFormat>>() // ipc-g13.ts ExportFormat
ok<Eq<H16.G13Api, G.G13Api>>() // ipc-g13.ts G13Api
import type * as H17 from './ipc-g14.ts'
ok<Eq<H17.ObservabilitySettings, G.ObservabilitySettings>>() // ipc-g14.ts ObservabilitySettings
ok<Eq<H17.ExportStatus, G.ExportStatus>>() // ipc-g14.ts ExportStatus
ok<Eq<H17.ProviderState, G.ProviderState>>() // ipc-g14.ts ProviderState
ok<Eq<H17.HealthInfo, G.HealthInfo>>() // ipc-g14.ts HealthInfo
ok<Eq<H17.SlowCall, G.SlowCall>>() // ipc-g14.ts SlowCall
ok<Eq<H17.Overview, G.Overview>>() // ipc-g14.ts Overview
ok<Eq<H17.CostReport, G.CostReport>>() // ipc-g14.ts CostReport
ok<Eq<H17.ProcessReport, G.ProcessReport>>() // ipc-g14.ts ProcessReport
ok<Eq<H17.G14Alert, G.G14Alert>>() // ipc-g14.ts G14Alert
ok<Eq<H17.ComposerStatus, G.ComposerStatus>>() // ipc-g14.ts ComposerStatus
ok<Eq<H17.G14Api, G.G14Api>>() // ipc-g14.ts G14Api
ok<Eq<H17.CostAttribution, G.CostAttribution>>() // ipc-g14.ts CostAttribution
ok<Eq<H17.ErrorBucket, G.ErrorBucket>>() // ipc-g14.ts ErrorBucket
ok<Eq<H17.ModelRecommendation, G.ModelRecommendation>>() // ipc-g14.ts ModelRecommendation
ok<Eq<H17.ModelStat, G.ModelStat>>() // ipc-g14.ts ModelStat
ok<Eq<H17.ProcessGroup, G.ProcessGroup>>() // ipc-g14.ts ProcessGroup
ok<Eq<H17.RawExchange, G.RawExchange>>() // ipc-g14.ts RawExchange
ok<Eq<H17.SpanStatus, G.SpanStatus>>() // ipc-g14.ts SpanStatus
ok<Eq<H17.SpendForecast, G.SpendForecast>>() // ipc-g14.ts SpendForecast
ok<Eq<H17.TelemetryEvent, G.TelemetryEvent>>() // ipc-g14.ts TelemetryEvent
ok<Eq<H17.ToolStat, G.ToolStat>>() // ipc-g14.ts ToolStat
ok<Eq<H17.TurnRecord, G.TurnRecord>>() // ipc-g14.ts TurnRecord
import type * as H18 from './ipc-g16.ts'
ok<Eq<H18.Distribution, G.Distribution>>() // ipc-g16.ts Distribution
ok<Eq<H18.UpdatePhase, G.UpdatePhase>>() // ipc-g16.ts UpdatePhase
ok<Eq<H18.UpdateStatus, G.UpdateStatus>>() // ipc-g16.ts UpdateStatus
ok<Eq<H18.CliStatus, G.CliStatus>>() // ipc-g16.ts CliStatus
ok<Eq<H18.CrashInfo, G.CrashInfo>>() // ipc-g16.ts CrashInfo
ok<Eq<H18.ConfigImportResult, G.ConfigImportResult>>() // ipc-g16.ts ConfigImportResult
ok<Eq<H18.CrashEvent, G.CrashEvent>>() // ipc-g16.ts CrashEvent
ok<Eq<H18.DumpInfo, G.DumpInfo>>() // ipc-g16.ts DumpInfo
ok<Eq<H18.ImportSummary, G.ImportSummary>>() // ipc-g16.ts ImportSummary
import type * as H19 from './ipc-g2.ts'
ok<Eq<H19.G2ImageInput, G.G2ImageInput>>() // ipc-g2.ts G2ImageInput
ok<Eq<H19.G2SendPayload, G.G2SendPayload>>() // ipc-g2.ts G2SendPayload
ok<Eq<H19.G2SendResult, G.G2SendResult>>() // ipc-g2.ts G2SendResult
ok<Eq<H19.G2FileHit, G.G2FileHit>>() // ipc-g2.ts G2FileHit
ok<Eq<H19.G2FileStat, G.G2FileStat>>() // ipc-g2.ts G2FileStat
ok<Eq<H19.G2DropResult, G.G2DropResult>>() // ipc-g2.ts G2DropResult
ok<Eq<H19.G2ModelCaps, G.G2ModelCaps>>() // ipc-g2.ts G2ModelCaps
ok<Eq<H19.G2EditPayload, G.G2EditPayload>>() // ipc-g2.ts G2EditPayload
ok<Eq<H19.G2RegeneratePayload, G.G2RegeneratePayload>>() // ipc-g2.ts G2RegeneratePayload
import type * as H20 from './ipc-g22.ts'
ok<Eq<H20.G22FileSupport, G.G22FileSupport>>() // ipc-g22.ts G22FileSupport
ok<Eq<H20.G22ModelSupport, G.G22ModelSupport>>() // ipc-g22.ts G22ModelSupport
ok<Eq<H20.ToolMode, G.ToolMode>>() // ipc-g22.ts ToolMode
import type * as H21 from './ipc-g35.ts'
ok<Eq<H21.VeniceWebView, G.VeniceWebView>>() // ipc-g35.ts VeniceWebView
ok<Eq<H21.VeniceWebLoginResult, G.VeniceWebLoginResult>>() // ipc-g35.ts VeniceWebLoginResult
ok<Eq<H21.G35Api, G.G35Api>>() // ipc-g35.ts G35Api
import type * as H22 from './ipc-g4.ts'
ok<Eq<H22.G4FileEntry, G.G4FileEntry>>() // ipc-g4.ts G4FileEntry
ok<Eq<H22.FileListing, G.FileListing>>() // ipc-g4.ts FileListing
ok<Eq<H22.FilePreview, G.FilePreview>>() // ipc-g4.ts FilePreview
ok<Eq<H22.SaveResult, G.SaveResult>>() // ipc-g4.ts SaveResult
ok<Eq<H22.RewindAction, G.RewindAction>>() // ipc-g4.ts RewindAction
ok<Eq<H22.RewindPreview, G.RewindPreview>>() // ipc-g4.ts RewindPreview
ok<Eq<H22.RewindChange, G.RewindChange>>() // ipc-g4.ts RewindChange
ok<Eq<H22.RewindResult, G.RewindResult>>() // ipc-g4.ts RewindResult
ok<Eq<H22.ExternalStatus, G.ExternalStatus>>() // ipc-g4.ts ExternalStatus
ok<Eq<H22.G4Event, G.G4Event>>() // ipc-g4.ts G4Event
ok<Eq<H22.OpenedWith, G.OpenedWith>>() // ipc-g4.ts OpenedWith
ok<Eq<H22.G4Api, G.G4Api>>() // ipc-g4.ts G4Api
ok<Eq<H22.CheckpointInfo, G.CheckpointInfo>>() // ipc-g4.ts CheckpointInfo
ok<Eq<H22.Diagnostic, G.Diagnostic>>() // ipc-g4.ts Diagnostic
ok<Eq<H22.DiagnosticsReport, G.DiagnosticsReport>>() // ipc-g4.ts DiagnosticsReport
ok<Eq<H22.EditorId, G.EditorId>>() // ipc-g4.ts EditorId
ok<Eq<H22.ToolReport, G.ToolReport>>() // ipc-g4.ts ToolReport
import type * as H23 from './ipc-g6.ts'
ok<Eq<H23.ContextView, G.ContextView>>() // ipc-g6.ts ContextView
ok<Eq<H23.CompactSettings, G.CompactSettings>>() // ipc-g6.ts CompactSettings
ok<Eq<H23.CompactSettingsPatch, G.CompactSettingsPatch>>() // ipc-g6.ts CompactSettingsPatch
ok<Eq<H23.ProjectMapInfo, G.ProjectMapInfo>>() // ipc-g6.ts ProjectMapInfo
ok<Eq<H23.CompactionEvent, G.CompactionEvent>>() // ipc-g6.ts CompactionEvent
ok<Eq<H23.CompactionResult, G.CompactionResult>>() // ipc-g6.ts CompactionResult
ok<Eq<H23.ContextInfo, G.ContextInfo>>() // ipc-g6.ts ContextInfo
ok<Eq<H23.InitProjectResult, G.InitProjectResult>>() // ipc-g6.ts InitProjectResult
ok<Eq<H23.MapProgress, G.MapProgress>>() // ipc-g6.ts MapProgress
ok<Eq<H23.ProjectMapStatus, G.ProjectMapStatus>>() // ipc-g6.ts ProjectMapStatus
import type * as H24 from './ipc-g9.ts'
ok<Eq<H24.KeyProfileView, G.KeyProfileView>>() // ipc-g9.ts KeyProfileView
ok<Eq<H24.ProviderView, G.ProviderView>>() // ipc-g9.ts ProviderView
ok<Eq<H24.ProvidersView, G.ProvidersView>>() // ipc-g9.ts ProvidersView
ok<Eq<H24.G9SettingsPatch, G.G9SettingsPatch>>() // ipc-g9.ts G9SettingsPatch
ok<Eq<H24.CostInfo, G.CostInfo>>() // ipc-g9.ts CostInfo
ok<Eq<H24.PullState, G.PullState>>() // ipc-g9.ts PullState
ok<Eq<H24.OllamaView, G.OllamaView>>() // ipc-g9.ts OllamaView
ok<Eq<H24.G9Api, G.G9Api>>() // ipc-g9.ts G9Api
ok<Eq<H24.ProbeResult, G.ProbeResult>>() // ipc-g9.ts ProbeResult
ok<Eq<H24.TurnCostInfo, G.TurnCostInfo>>() // ipc-g9.ts TurnCostInfo
import type * as H25 from './ipc-git.ts'
ok<Eq<H25.GitStatus, G.GitStatus>>() // ipc-git.ts GitStatus
ok<Eq<H25.GitBranches, G.GitBranches>>() // ipc-git.ts GitBranches
ok<Eq<H25.CreatePrInput, G.CreatePrInput>>() // ipc-git.ts CreatePrInput
ok<Eq<H25.CreatePrResult, G.CreatePrResult>>() // ipc-git.ts CreatePrResult
ok<Eq<H25.GitApi, G.GitApi>>() // ipc-git.ts GitApi
import type * as H26 from './ipc-graph.ts'
ok<Eq<H26.NodeKind, G.NodeKind>>() // ipc-graph.ts NodeKind
ok<Eq<H26.EdgeKindName, G.EdgeKindName>>() // ipc-graph.ts EdgeKindName
ok<Eq<H26.EdgeKind, G.EdgeKind>>() // ipc-graph.ts EdgeKind
ok<Eq<H26.Confidence, G.Confidence>>() // ipc-graph.ts Confidence
ok<Eq<H26.GraphStatus, G.GraphStatus>>() // ipc-graph.ts GraphStatus
ok<Eq<H26.StatusView, G.StatusView>>() // ipc-graph.ts StatusView
ok<Eq<H26.Community, G.Community>>() // ipc-graph.ts Community
ok<Eq<H26.SnapshotNode, G.SnapshotNode>>() // ipc-graph.ts SnapshotNode
ok<Eq<H26.SnapshotEdge, G.SnapshotEdge>>() // ipc-graph.ts SnapshotEdge
ok<Eq<H26.Snapshot, G.Snapshot>>() // ipc-graph.ts Snapshot
ok<Eq<H26.NodeRef, G.NodeRef>>() // ipc-graph.ts NodeRef
ok<Eq<H26.GraphNode, G.GraphNode>>() // ipc-graph.ts GraphNode
ok<Eq<H26.Link, G.Link>>() // ipc-graph.ts Link
ok<Eq<H26.Explanation, G.Explanation>>() // ipc-graph.ts Explanation
ok<Eq<H26.PathStep, G.PathStep>>() // ipc-graph.ts PathStep
ok<Eq<H26.QueryHit, G.QueryHit>>() // ipc-graph.ts QueryHit
ok<Eq<H26.Hotspot, G.Hotspot>>() // ipc-graph.ts Hotspot
ok<Eq<H26.Impact, G.Impact>>() // ipc-graph.ts Impact
ok<Eq<H26.GraphDiff, G.GraphDiff>>() // ipc-graph.ts GraphDiff
ok<Eq<H26.ExportFormat, G.ExportFormat__graph>>() // ipc-graph.ts ExportFormat
ok<Eq<H26.ExportReport, G.ExportReport>>() // ipc-graph.ts ExportReport
ok<Eq<H26.UpdateStats, G.UpdateStats>>() // ipc-graph.ts UpdateStats
ok<Eq<H26.GraphUpdatedEvent, G.GraphUpdatedEvent>>() // ipc-graph.ts GraphUpdatedEvent
ok<Eq<H26.GraphTouchedEvent, G.GraphTouchedEvent>>() // ipc-graph.ts GraphTouchedEvent
ok<Eq<H26.GraphApi, G.GraphApi>>() // ipc-graph.ts GraphApi
import type * as H27 from './ipc-intent.ts'
ok<Eq<H27.IntentRun, G.IntentRun>>() // ipc-intent.ts IntentRun
ok<Eq<H27.IntentVia, G.IntentVia>>() // ipc-intent.ts IntentVia
ok<Eq<H27.IntentPlan, G.IntentPlan>>() // ipc-intent.ts IntentPlan
ok<Eq<H27.IntentStep, G.IntentStep>>() // ipc-intent.ts IntentStep
ok<Eq<H27.IntentDecision, G.IntentDecision>>() // ipc-intent.ts IntentDecision
ok<Eq<H27.IntentRouteContext, G.IntentRouteContext>>() // ipc-intent.ts IntentRouteContext
ok<Eq<H27.IntentRunResult, G.IntentRunResult>>() // ipc-intent.ts IntentRunResult
ok<Eq<H27.IntentPhrase, G.IntentPhrase>>() // ipc-intent.ts IntentPhrase
ok<Eq<H27.IntentEntry, G.IntentEntry>>() // ipc-intent.ts IntentEntry
ok<Eq<H27.IntentPattern, G.IntentPattern>>() // ipc-intent.ts IntentPattern
ok<Eq<H27.IntentApi, G.IntentApi>>() // ipc-intent.ts IntentApi
ok<Eq<H27.ToolEntry, G.ToolEntry>>() // ipc-intent.ts ToolEntry
ok<Eq<H27.TypedTool, G.TypedTool>>() // ipc-intent.ts TypedTool
ok<Eq<H27.ToolHelp, G.ToolHelp>>() // ipc-intent.ts ToolHelp
ok<Eq<H27.ToolsApi, G.ToolsApi>>() // ipc-intent.ts ToolsApi
import type * as H28 from './ipc-korfolder.ts'
ok<Eq<H28.KorFolderItem, G.KorFolderItem>>() // ipc-korfolder.ts KorFolderItem
ok<Eq<H28.KorFolderMount, G.KorFolderMount>>() // ipc-korfolder.ts KorFolderMount
ok<Eq<H28.KorFolderStatus, G.KorFolderStatus>>() // ipc-korfolder.ts KorFolderStatus
ok<Eq<H28.KorFolderSync, G.KorFolderSync>>() // ipc-korfolder.ts KorFolderSync
ok<Eq<H28.KorFolderRemove, G.KorFolderRemove>>() // ipc-korfolder.ts KorFolderRemove
ok<Eq<H28.KorFolderTask, G.KorFolderTask>>() // ipc-korfolder.ts KorFolderTask
ok<Eq<H28.KorFolderPass, G.KorFolderPass>>() // ipc-korfolder.ts KorFolderPass
ok<Eq<H28.KorFolderApi, G.KorFolderApi>>() // ipc-korfolder.ts KorFolderApi
import type * as H29 from './ipc-logs.ts'
ok<Eq<H29.LogEntry, G.LogEntry>>() // ipc-logs.ts LogEntry
ok<Eq<H29.LogsApi, G.LogsApi>>() // ipc-logs.ts LogsApi
import type * as H30 from './ipc-mcp.ts'
ok<Eq<H30.McpServerView, G.McpServerView>>() // ipc-mcp.ts McpServerView
ok<Eq<H30.McpApi, G.McpApi>>() // ipc-mcp.ts McpApi
import type * as H31 from './ipc-mcpgen.ts'
ok<Eq<H31.McpGenKind, G.McpGenKind>>() // ipc-mcpgen.ts McpGenKind
ok<Eq<H31.McpGenRequest, G.McpGenRequest>>() // ipc-mcpgen.ts McpGenRequest
ok<Eq<H31.McpGenAdded, G.McpGenAdded>>() // ipc-mcpgen.ts McpGenAdded
ok<Eq<H31.McpTestReport, G.McpTestReport>>() // ipc-mcpgen.ts McpTestReport
ok<Eq<H31.McpGenApi, G.McpGenApi>>() // ipc-mcpgen.ts McpGenApi
import type * as H32 from './ipc-memory.ts'
ok<Eq<H32.KnowledgeRecord, G.KnowledgeRecord>>() // ipc-memory.ts KnowledgeRecord
ok<Eq<H32.RecallHit, G.RecallHit>>() // ipc-memory.ts RecallHit
ok<Eq<H32.Decision, G.Decision>>() // ipc-memory.ts Decision
ok<Eq<H32.MemorySection, G.MemorySection>>() // ipc-memory.ts MemorySection
ok<Eq<H32.MemoryDocFile, G.MemoryDocFile>>() // ipc-memory.ts MemoryDocFile
ok<Eq<H32.MemoryOverview, G.MemoryOverview__memory>>() // ipc-memory.ts MemoryOverview
ok<Eq<H32.MemoryApi, G.MemoryApi>>() // ipc-memory.ts MemoryApi
ok<Eq<H32.ProjMemApi, G.ProjMemApi>>() // ipc-memory.ts ProjMemApi
import type * as H33 from './ipc-metrics.ts'
ok<Eq<H33.Grade, G.Grade>>() // ipc-metrics.ts Grade
ok<Eq<H33.DimensionKey, G.DimensionKey>>() // ipc-metrics.ts DimensionKey
ok<Eq<H33.Metric, G.Metric>>() // ipc-metrics.ts Metric
ok<Eq<H33.Dimension, G.Dimension>>() // ipc-metrics.ts Dimension
ok<Eq<H33.QualityTrend, G.QualityTrend>>() // ipc-metrics.ts QualityTrend
ok<Eq<H33.QualityReport, G.QualityReport>>() // ipc-metrics.ts QualityReport
ok<Eq<H33.QualitySnapshot, G.QualitySnapshot>>() // ipc-metrics.ts QualitySnapshot
ok<Eq<H33.TestBasis, G.TestBasis>>() // ipc-metrics.ts TestBasis
ok<Eq<H33.DebtItem, G.DebtItem>>() // ipc-metrics.ts DebtItem
ok<Eq<H33.GrowthBasis, G.GrowthBasis>>() // ipc-metrics.ts GrowthBasis
ok<Eq<H33.Debt, G.Debt>>() // ipc-metrics.ts Debt
ok<Eq<H33.Convention, G.Convention>>() // ipc-metrics.ts Convention
ok<Eq<H33.StyleProfile, G.StyleProfile>>() // ipc-metrics.ts StyleProfile
ok<Eq<H33.StatsLines, G.StatsLines>>() // ipc-metrics.ts StatsLines
ok<Eq<H33.StatsWeek, G.StatsWeek>>() // ipc-metrics.ts StatsWeek
ok<Eq<H33.TeamRow, G.TeamRow>>() // ipc-metrics.ts TeamRow
ok<Eq<H33.DevStats, G.DevStats>>() // ipc-metrics.ts DevStats
ok<Eq<H33.Endpoint, G.Endpoint>>() // ipc-metrics.ts Endpoint
ok<Eq<H33.ImpactReport, G.ImpactReport>>() // ipc-metrics.ts ImpactReport
ok<Eq<H33.DeadConfidence, G.DeadConfidence>>() // ipc-metrics.ts DeadConfidence
ok<Eq<H33.DeadReason, G.DeadReason>>() // ipc-metrics.ts DeadReason
ok<Eq<H33.DeadItem, G.DeadItem>>() // ipc-metrics.ts DeadItem
ok<Eq<H33.DeadReport, G.DeadReport>>() // ipc-metrics.ts DeadReport
ok<Eq<H33.CmdRun, G.CmdRun>>() // ipc-metrics.ts CmdRun
ok<Eq<H33.PrunePlan, G.PrunePlan>>() // ipc-metrics.ts PrunePlan
ok<Eq<H33.Author, G.Author>>() // ipc-metrics.ts Author
ok<Eq<H33.CommitRef, G.CommitRef>>() // ipc-metrics.ts CommitRef
ok<Eq<H33.DangerZone, G.DangerZone>>() // ipc-metrics.ts DangerZone
ok<Eq<H33.Archaeology, G.Archaeology>>() // ipc-metrics.ts Archaeology
ok<Eq<H33.ExplainLevel, G.ExplainLevel>>() // ipc-metrics.ts ExplainLevel
ok<Eq<H33.ImpactMode, G.ImpactMode>>() // ipc-metrics.ts ImpactMode
ok<Eq<H33.MetricsApi, G.MetricsApi>>() // ipc-metrics.ts MetricsApi
import type * as H34 from './ipc-notes.ts'
ok<Eq<H34.NotesApi, G.NotesApi>>() // ipc-notes.ts NotesApi
import type * as H35 from './ipc-offline.ts'
ok<Eq<H35.OfflineServer, G.OfflineServer>>() // ipc-offline.ts OfflineServer
ok<Eq<H35.OfflineModel, G.OfflineModel>>() // ipc-offline.ts OfflineModel
ok<Eq<H35.OfflineMissing, G.OfflineMissing>>() // ipc-offline.ts OfflineMissing
ok<Eq<H35.OfflineStatus, G.OfflineStatus>>() // ipc-offline.ts OfflineStatus
ok<Eq<H35.OfflineInitOptions, G.OfflineInitOptions>>() // ipc-offline.ts OfflineInitOptions
ok<Eq<H35.OfflineDownload, G.OfflineDownload>>() // ipc-offline.ts OfflineDownload
ok<Eq<H35.OfflinePlan, G.OfflinePlan>>() // ipc-offline.ts OfflinePlan
ok<Eq<H35.OfflineStep, G.OfflineStep>>() // ipc-offline.ts OfflineStep
ok<Eq<H35.OfflineEvent, G.OfflineEvent>>() // ipc-offline.ts OfflineEvent
ok<Eq<H35.OfflineDone, G.OfflineDone>>() // ipc-offline.ts OfflineDone
ok<Eq<H35.OfflineBundleOptions, G.OfflineBundleOptions>>() // ipc-offline.ts OfflineBundleOptions
ok<Eq<H35.OfflineApi, G.OfflineApi>>() // ipc-offline.ts OfflineApi
import type * as H36 from './ipc-osextra.ts'
ok<Eq<H36.OsName, G.OsName>>() // ipc-osextra.ts OsName
ok<Eq<H36.OsExtraKind, G.OsExtraKind>>() // ipc-osextra.ts OsExtraKind
ok<Eq<H36.OsExtraStatus, G.OsExtraStatus>>() // ipc-osextra.ts OsExtraStatus
ok<Eq<H36.OsExtraList, G.OsExtraList>>() // ipc-osextra.ts OsExtraList
ok<Eq<H36.PortalOutcome, G.PortalOutcome>>() // ipc-osextra.ts PortalOutcome
ok<Eq<H36.OsExtraApi, G.OsExtraApi>>() // ipc-osextra.ts OsExtraApi
import type * as H37 from './ipc-osint.ts'
ok<Eq<H37.OsintOrigin, G.OsintOrigin>>() // ipc-osint.ts OsintOrigin
ok<Eq<H37.QuickTask, G.QuickTask>>() // ipc-osint.ts QuickTask
ok<Eq<H37.OsintRequest, G.OsintRequest>>() // ipc-osint.ts OsintRequest
ok<Eq<H37.HotkeyStatus, G.HotkeyStatus>>() // ipc-osint.ts HotkeyStatus
ok<Eq<H37.MenuStatus, G.MenuStatus>>() // ipc-osint.ts MenuStatus
ok<Eq<H37.OsintApi, G.OsintApi>>() // ipc-osint.ts OsintApi
import type * as H38 from './ipc-packages.ts'
ok<Eq<H38.PackageTool, G.PackageTool>>() // ipc-packages.ts PackageTool
ok<Eq<H38.PackageView, G.PackageView>>() // ipc-packages.ts PackageView
ok<Eq<H38.PackagesList, G.PackagesList>>() // ipc-packages.ts PackagesList
ok<Eq<H38.PackagesAffected, G.PackagesAffected>>() // ipc-packages.ts PackagesAffected
ok<Eq<H38.PackageDeps, G.PackageDeps>>() // ipc-packages.ts PackageDeps
ok<Eq<H38.PackageCostRow, G.PackageCostRow>>() // ipc-packages.ts PackageCostRow
ok<Eq<H38.PackagesApi, G.PackagesApi>>() // ipc-packages.ts PackagesApi
import type * as H39 from './ipc-perf.ts'
ok<Eq<H39.PerfUnit, G.PerfUnit>>() // ipc-perf.ts PerfUnit
ok<Eq<H39.PerfNodeLink, G.PerfNodeLink>>() // ipc-perf.ts PerfNodeLink
ok<Eq<H39.PerfRow, G.PerfRow>>() // ipc-perf.ts PerfRow
ok<Eq<H39.PerfLoadStats, G.PerfLoadStats>>() // ipc-perf.ts PerfLoadStats
ok<Eq<H39.PerfCompareRow, G.PerfCompareRow>>() // ipc-perf.ts PerfCompareRow
ok<Eq<H39.PerfReport, G.PerfReport>>() // ipc-perf.ts PerfReport
ok<Eq<H39.PerfSummary, G.PerfSummary>>() // ipc-perf.ts PerfSummary
ok<Eq<H39.PerfRunOptions, G.PerfRunOptions>>() // ipc-perf.ts PerfRunOptions
ok<Eq<H39.PerfApi, G.PerfApi>>() // ipc-perf.ts PerfApi
import type * as H40 from './ipc-plugins.ts'
ok<Eq<H40.CatalogItem, G.CatalogItem>>() // ipc-plugins.ts CatalogItem
ok<Eq<H40.StoreItem, G.StoreItem>>() // ipc-plugins.ts StoreItem
ok<Eq<H40.PluginsApi, G.PluginsApi>>() // ipc-plugins.ts PluginsApi
import type * as H41 from './ipc-prefs.ts'
ok<Eq<H41.AutoRow, G.AutoRow>>() // ipc-prefs.ts AutoRow
ok<Eq<H41.LineWidth, G.LineWidth>>() // ipc-prefs.ts LineWidth
ok<Eq<H41.Density, G.Density>>() // ipc-prefs.ts Density
ok<Eq<H41.DiffViewMode, G.DiffViewMode>>() // ipc-prefs.ts DiffViewMode
ok<Eq<H41.ToolOutput, G.ToolOutput>>() // ipc-prefs.ts ToolOutput
ok<Eq<H41.KeyMode, G.KeyMode>>() // ipc-prefs.ts KeyMode
ok<Eq<H41.UiPrefs, G.UiPrefs>>() // ipc-prefs.ts UiPrefs
ok<Eq<H41.AskThreshold, G.AskThreshold>>() // ipc-prefs.ts AskThreshold
ok<Eq<H41.AgentPrefs, G.AgentPrefs>>() // ipc-prefs.ts AgentPrefs
ok<Eq<H41.Keymap, G.Keymap>>() // ipc-prefs.ts Keymap
ok<Eq<H41.DoctorFix, G.DoctorFix>>() // ipc-prefs.ts DoctorFix
ok<Eq<H41.PrefsApi, G.PrefsApi>>() // ipc-prefs.ts PrefsApi
ok<Eq<H41.QuestionAnswer, G.QuestionAnswer>>() // ipc-prefs.ts QuestionAnswer
ok<Eq<H41.CompactAnswer, G.CompactAnswer>>() // ipc-prefs.ts CompactAnswer
ok<Eq<H41.QuestionEvent, G.QuestionEvent>>() // ipc-prefs.ts QuestionEvent
ok<Eq<H41.CompactItem, G.CompactItem>>() // ipc-prefs.ts CompactItem
ok<Eq<H41.CompactPreviewEvent, G.CompactPreviewEvent>>() // ipc-prefs.ts CompactPreviewEvent
ok<Eq<H41.PlanInfo, G.PlanInfo>>() // ipc-prefs.ts PlanInfo
ok<Eq<H41.PlanApi, G.PlanApi>>() // ipc-prefs.ts PlanApi
ok<Eq<H41.SessionEntry, G.SessionEntry>>() // ipc-prefs.ts SessionEntry
ok<Eq<H41.SessionHit, G.SessionHit>>() // ipc-prefs.ts SessionHit
ok<Eq<H41.SessionFilter, G.SessionFilter>>() // ipc-prefs.ts SessionFilter
ok<Eq<H41.SessionColor, G.SessionColor>>() // ipc-prefs.ts SessionColor
ok<Eq<H41.SessionsApi, G.SessionsApi>>() // ipc-prefs.ts SessionsApi
import type * as H42 from './ipc-preview.ts'
ok<Eq<H42.DevServerStatus, G.DevServerStatus>>() // ipc-preview.ts DevServerStatus
ok<Eq<H42.PreviewBounds, G.PreviewBounds>>() // ipc-preview.ts PreviewBounds
ok<Eq<H42.PreviewSelection, G.PreviewSelection>>() // ipc-preview.ts PreviewSelection
ok<Eq<H42.PreviewEvent, G.PreviewEvent>>() // ipc-preview.ts PreviewEvent
ok<Eq<H42.PreviewApi, G.PreviewApi>>() // ipc-preview.ts PreviewApi
import type * as H43 from './ipc-providers.ts'
ok<Eq<H43.ProviderErrorView, G.ProviderErrorView>>() // ipc-providers.ts ProviderErrorView
ok<Eq<H43.ProviderCheck, G.ProviderCheck>>() // ipc-providers.ts ProviderCheck
ok<Eq<H43.ProviderTestResult, G.ProviderTestResult>>() // ipc-providers.ts ProviderTestResult
ok<Eq<H43.ProviderStatusRow, G.ProviderStatusRow>>() // ipc-providers.ts ProviderStatusRow
ok<Eq<H43.ProviderApi, G.ProviderApi>>() // ipc-providers.ts ProviderApi
ok<Eq<H43.CatalogEntry, G.CatalogEntry__providers>>() // ipc-providers.ts CatalogEntry
ok<Eq<H43.CatalogResult, G.CatalogResult>>() // ipc-providers.ts CatalogResult
ok<Eq<H43.CatalogApi, G.CatalogApi>>() // ipc-providers.ts CatalogApi
ok<Eq<H43.ProxyModelUsage, G.ProxyModelUsage>>() // ipc-providers.ts ProxyModelUsage
ok<Eq<H43.ProxyCredential, G.ProxyCredential>>() // ipc-providers.ts ProxyCredential
ok<Eq<H43.ProxyStatus, G.ProxyStatus>>() // ipc-providers.ts ProxyStatus
ok<Eq<H43.ProxyStart, G.ProxyStart>>() // ipc-providers.ts ProxyStart
ok<Eq<H43.ProxyConfig, G.ProxyConfig>>() // ipc-providers.ts ProxyConfig
ok<Eq<H43.ProxyApi, G.ProxyApi>>() // ipc-providers.ts ProxyApi
ok<Eq<H43.ComponentView, G.ComponentView>>() // ipc-providers.ts ComponentView
ok<Eq<H43.ComponentsApi, G.ComponentsApi>>() // ipc-providers.ts ComponentsApi
ok<Eq<H43.RuntimeTick, G.RuntimeTick>>() // ipc-providers.ts RuntimeTick
ok<Eq<H43.MediaJob, G.MediaJob>>() // ipc-providers.ts MediaJob
ok<Eq<H43.HfModel, G.HfModel>>() // ipc-providers.ts HfModel
ok<Eq<H43.HfInfo, G.HfInfo>>() // ipc-providers.ts HfInfo
ok<Eq<H43.MediaProgress, G.MediaProgress>>() // ipc-providers.ts MediaProgress
ok<Eq<H43.HfProgress, G.HfProgress>>() // ipc-providers.ts HfProgress
ok<Eq<H43.MediaApi, G.MediaApi>>() // ipc-providers.ts MediaApi
import type * as H44 from './ipc-remote.ts'
ok<Eq<H44.RemoteHost, G.RemoteHost>>() // ipc-remote.ts RemoteHost
ok<Eq<H44.RemoteTask, G.RemoteTask__remote>>() // ipc-remote.ts RemoteTask
ok<Eq<H44.RemoteConnected, G.RemoteConnected>>() // ipc-remote.ts RemoteConnected
ok<Eq<H44.RemoteStatus, G.RemoteStatus>>() // ipc-remote.ts RemoteStatus
ok<Eq<H44.RemoteWireEvent, G.RemoteWireEvent>>() // ipc-remote.ts RemoteWireEvent
ok<Eq<H44.RemoteEvent, G.RemoteEvent>>() // ipc-remote.ts RemoteEvent
ok<Eq<H44.InfraService, G.InfraService>>() // ipc-remote.ts InfraService
ok<Eq<H44.InfraSnapshot, G.InfraSnapshot>>() // ipc-remote.ts InfraSnapshot
ok<Eq<H44.SudoOffer, G.SudoOffer>>() // ipc-remote.ts SudoOffer
ok<Eq<H44.Discovery, G.Discovery>>() // ipc-remote.ts Discovery
ok<Eq<H44.RemoteApi, G.RemoteApi>>() // ipc-remote.ts RemoteApi
import type * as H45 from './ipc-replay.ts'
ok<Eq<H45.ReplaySummary, G.ReplaySummary>>() // ipc-replay.ts ReplaySummary
ok<Eq<H45.ReplayStep, G.ReplayStep>>() // ipc-replay.ts ReplayStep
ok<Eq<H45.ReplayFile, G.ReplayFile>>() // ipc-replay.ts ReplayFile
ok<Eq<H45.ReplayEvent, G.ReplayEvent>>() // ipc-replay.ts ReplayEvent
ok<Eq<H45.ReplayRecording, G.ReplayRecording>>() // ipc-replay.ts ReplayRecording
ok<Eq<H45.ReplayApi, G.ReplayApi>>() // ipc-replay.ts ReplayApi
ok<Eq<H45.EnvTool, G.EnvTool>>() // ipc-replay.ts EnvTool
ok<Eq<H45.EnvKind, G.EnvKind>>() // ipc-replay.ts EnvKind
ok<Eq<H45.EnvVersion, G.EnvVersion>>() // ipc-replay.ts EnvVersion
ok<Eq<H45.EnvStatus, G.EnvStatus>>() // ipc-replay.ts EnvStatus
ok<Eq<H45.EnvApi, G.EnvApi>>() // ipc-replay.ts EnvApi
import type * as H46 from './ipc-review.ts'
ok<Eq<H46.ReviewFile, G.ReviewFile>>() // ipc-review.ts ReviewFile
ok<Eq<H46.FileDecision, G.FileDecision>>() // ipc-review.ts FileDecision
ok<Eq<H46.FileDecisions, G.FileDecisions>>() // ipc-review.ts FileDecisions
ok<Eq<H46.ConfirmReviewFields, G.ConfirmReviewFields>>() // ipc-review.ts ConfirmReviewFields
import type * as H47 from './ipc-routines.ts'
ok<Eq<H47.RoutineInput, G.RoutineInput>>() // ipc-routines.ts RoutineInput
ok<Eq<H47.RoutinesApi, G.RoutinesApi>>() // ipc-routines.ts RoutinesApi
import type * as H48 from './ipc-security.ts'
ok<Eq<H48.VaultList, G.VaultList>>() // ipc-security.ts VaultList
ok<Eq<H48.VaultApi, G.VaultApi>>() // ipc-security.ts VaultApi
ok<Eq<H48.VaultImport, G.VaultImport>>() // ipc-security.ts VaultImport
ok<Eq<H48.AuditQuery, G.AuditQuery>>() // ipc-security.ts AuditQuery
ok<Eq<H48.AuditEntry, G.AuditEntry>>() // ipc-security.ts AuditEntry
ok<Eq<H48.AuditPage, G.AuditPage>>() // ipc-security.ts AuditPage
ok<Eq<H48.AuditHead, G.AuditHead>>() // ipc-security.ts AuditHead
ok<Eq<H48.AuditVerify, G.AuditVerify>>() // ipc-security.ts AuditVerify
ok<Eq<H48.AuditReport, G.AuditReport>>() // ipc-security.ts AuditReport
ok<Eq<H48.AuditApi, G.AuditApi>>() // ipc-security.ts AuditApi
ok<Eq<H48.SandboxMode, G.SandboxMode>>() // ipc-security.ts SandboxMode
ok<Eq<H48.SandboxPreference, G.SandboxPreference>>() // ipc-security.ts SandboxPreference
ok<Eq<H48.SandboxStatus, G.SandboxStatus__security>>() // ipc-security.ts SandboxStatus
ok<Eq<H48.SandboxProbe, G.SandboxProbe>>() // ipc-security.ts SandboxProbe
ok<Eq<H48.SandboxApi, G.SandboxApi>>() // ipc-security.ts SandboxApi
import type * as H49 from './ipc-seo.ts'
ok<Eq<H49.SeoSeverity, G.SeoSeverity>>() // ipc-seo.ts SeoSeverity
ok<Eq<H49.SeoDepth, G.SeoDepth>>() // ipc-seo.ts SeoDepth
ok<Eq<H49.SeoFinding, G.SeoFinding>>() // ipc-seo.ts SeoFinding
ok<Eq<H49.SeoReport, G.SeoReport>>() // ipc-seo.ts SeoReport
ok<Eq<H49.SeoDiff, G.SeoDiff>>() // ipc-seo.ts SeoDiff
ok<Eq<H49.SeoRich, G.SeoRich>>() // ipc-seo.ts SeoRich
ok<Eq<H49.SeoAuditResult, G.SeoAuditResult>>() // ipc-seo.ts SeoAuditResult
ok<Eq<H49.SeoGscStatus, G.SeoGscStatus>>() // ipc-seo.ts SeoGscStatus
ok<Eq<H49.SeoRow, G.SeoRow>>() // ipc-seo.ts SeoRow
ok<Eq<H49.SeoDrop, G.SeoDrop>>() // ipc-seo.ts SeoDrop
ok<Eq<H49.SeoContentStage, G.SeoContentStage>>() // ipc-seo.ts SeoContentStage
ok<Eq<H49.SeoContentState, G.SeoContentState>>() // ipc-seo.ts SeoContentState
ok<Eq<H49.SeoContentFiles, G.SeoContentFiles>>() // ipc-seo.ts SeoContentFiles
ok<Eq<H49.SeoMetaRow, G.SeoMetaRow>>() // ipc-seo.ts SeoMetaRow
ok<Eq<H49.SeoMetaEstimate, G.SeoMetaEstimate>>() // ipc-seo.ts SeoMetaEstimate
ok<Eq<H49.SeoApplied, G.SeoApplied>>() // ipc-seo.ts SeoApplied
ok<Eq<H49.SeoBotStats, G.SeoBotStats>>() // ipc-seo.ts SeoBotStats
ok<Eq<H49.SeoLogStats, G.SeoLogStats>>() // ipc-seo.ts SeoLogStats
ok<Eq<H49.SeoLogsResult, G.SeoLogsResult>>() // ipc-seo.ts SeoLogsResult
ok<Eq<H49.SeoCompetitor, G.SeoCompetitor>>() // ipc-seo.ts SeoCompetitor
ok<Eq<H49.SeoCompDiff, G.SeoCompDiff>>() // ipc-seo.ts SeoCompDiff
ok<Eq<H49.SeoAgentEvent, G.SeoAgentEvent>>() // ipc-seo.ts SeoAgentEvent
ok<Eq<H49.SeoPriority, G.SeoPriority>>() // ipc-seo.ts SeoPriority
ok<Eq<H49.SeoTeamReport, G.SeoTeamReport>>() // ipc-seo.ts SeoTeamReport
ok<Eq<H49.SeoOpportunity, G.SeoOpportunity>>() // ipc-seo.ts SeoOpportunity
ok<Eq<H49.SeoBingStat, G.SeoBingStat>>() // ipc-seo.ts SeoBingStat
ok<Eq<H49.SeoVerified, G.SeoVerified>>() // ipc-seo.ts SeoVerified
ok<Eq<H49.SeoProgress, G.SeoProgress>>() // ipc-seo.ts SeoProgress
ok<Eq<H49.SeoApi, G.SeoApi>>() // ipc-seo.ts SeoApi
import type * as H50 from './ipc-shell.ts'
ok<Eq<H50.ShellFailure, G.ShellFailure>>() // ipc-shell.ts ShellFailure
ok<Eq<H50.ShellChange, G.ShellChange>>() // ipc-shell.ts ShellChange
ok<Eq<H50.ShellStatus, G.ShellStatus>>() // ipc-shell.ts ShellStatus
ok<Eq<H50.ShellApi, G.ShellApi>>() // ipc-shell.ts ShellApi
ok<Eq<H50.CommitProposal, G.CommitProposal>>() // ipc-shell.ts CommitProposal
ok<Eq<H50.GitflowApi, G.GitflowApi>>() // ipc-shell.ts GitflowApi
import type * as H51 from './ipc-shellui.ts'
ok<Eq<H51.NotificationCategory, G.NotificationCategory>>() // ipc-shellui.ts NotificationCategory
ok<Eq<H51.KorNotification, G.KorNotification>>() // ipc-shellui.ts KorNotification
ok<Eq<H51.Alert, G.Alert>>() // ipc-shellui.ts Alert
ok<Eq<H51.EtaScope, G.EtaScope>>() // ipc-shellui.ts EtaScope
ok<Eq<H51.Eta, G.Eta>>() // ipc-shellui.ts Eta
ok<Eq<H51.RunningSession, G.RunningSession>>() // ipc-shellui.ts RunningSession
ok<Eq<H51.ApprovalSource, G.ApprovalSource>>() // ipc-shellui.ts ApprovalSource
ok<Eq<H51.Risk, G.Risk>>() // ipc-shellui.ts Risk
ok<Eq<H51.QueueItem, G.QueueItem>>() // ipc-shellui.ts QueueItem
ok<Eq<H51.QueueGroup, G.QueueGroup>>() // ipc-shellui.ts QueueGroup
ok<Eq<H51.DaemonJob, G.DaemonJob>>() // ipc-shellui.ts DaemonJob
ok<Eq<H51.RemoteTask, G.RemoteTask>>() // ipc-shellui.ts RemoteTask
ok<Eq<H51.Overview, G.Overview__shellui>>() // ipc-shellui.ts Overview
ok<Eq<H51.ShellState, G.ShellState>>() // ipc-shellui.ts ShellState
ok<Eq<H51.ShellSettings, G.ShellSettings>>() // ipc-shellui.ts ShellSettings
ok<Eq<H51.ShellSettingKey, G.ShellSettingKey>>() // ipc-shellui.ts ShellSettingKey
ok<Eq<H51.ShellUiApi, G.ShellUiApi>>() // ipc-shellui.ts ShellUiApi
import type * as H52 from './ipc-tags.ts'
ok<Eq<H52.Tag, G.Tag>>() // ipc-tags.ts Tag
ok<Eq<H52.TagsApi, G.TagsApi>>() // ipc-tags.ts TagsApi
import type * as H53 from './ipc-tasks.ts'
ok<Eq<H53.TaskStatus, G.TaskStatus>>() // ipc-tasks.ts TaskStatus
ok<Eq<H53.SubStatus, G.SubStatus>>() // ipc-tasks.ts SubStatus
ok<Eq<H53.TaskTestRun, G.TaskTestRun>>() // ipc-tasks.ts TaskTestRun
ok<Eq<H53.SubTask, G.SubTask>>() // ipc-tasks.ts SubTask
ok<Eq<H53.ConflictFile, G.ConflictFile>>() // ipc-tasks.ts ConflictFile
ok<Eq<H53.TaskConflict, G.TaskConflict>>() // ipc-tasks.ts TaskConflict
ok<Eq<H53.TaskSettings, G.TaskSettings>>() // ipc-tasks.ts TaskSettings
ok<Eq<H53.TaskState, G.TaskState>>() // ipc-tasks.ts TaskState
ok<Eq<H53.TaskStartOptions, G.TaskStartOptions>>() // ipc-tasks.ts TaskStartOptions
ok<Eq<H53.TasksApi, G.TasksApi>>() // ipc-tasks.ts TasksApi
import type * as H54 from './ipc-team.ts'
ok<Eq<H54.TeamStatus, G.TeamStatus>>() // ipc-team.ts TeamStatus
ok<Eq<H54.TeamCheck, G.TeamCheck>>() // ipc-team.ts TeamCheck
ok<Eq<H54.TeamApi, G.TeamApi>>() // ipc-team.ts TeamApi
import type * as H55 from './ipc-teams.ts'
ok<Eq<H55.TeamRoleView, G.TeamRoleView>>() // ipc-teams.ts TeamRoleView
ok<Eq<H55.TeamView, G.TeamView>>() // ipc-teams.ts TeamView
ok<Eq<H55.TeamsApi, G.TeamsApi>>() // ipc-teams.ts TeamsApi
import type * as H56 from './ipc-templates.ts'
ok<Eq<H56.TemplateInputKind, G.TemplateInputKind>>() // ipc-templates.ts TemplateInputKind
ok<Eq<H56.TemplateInput, G.TemplateInput>>() // ipc-templates.ts TemplateInput
ok<Eq<H56.TaskTemplate, G.TaskTemplate>>() // ipc-templates.ts TaskTemplate
ok<Eq<H56.RenderedTemplate, G.RenderedTemplate>>() // ipc-templates.ts RenderedTemplate
ok<Eq<H56.TemplatesApi, G.TemplatesApi>>() // ipc-templates.ts TemplatesApi
import type * as H57 from './ipc-terminal.ts'
ok<Eq<H57.TerminalRunResult, G.TerminalRunResult>>() // ipc-terminal.ts TerminalRunResult
ok<Eq<H57.TerminalApi, G.TerminalApi>>() // ipc-terminal.ts TerminalApi
import type * as H58 from './ipc-triggers.ts'
ok<Eq<H58.TriggerEvent, G.TriggerEvent>>() // ipc-triggers.ts TriggerEvent
ok<Eq<H58.TriggerApproval, G.TriggerApproval>>() // ipc-triggers.ts TriggerApproval
ok<Eq<H58.TriggerRow, G.TriggerRow>>() // ipc-triggers.ts TriggerRow
ok<Eq<H58.TriggerInput, G.TriggerInput>>() // ipc-triggers.ts TriggerInput
ok<Eq<H58.JobStatus, G.JobStatus>>() // ipc-triggers.ts JobStatus
ok<Eq<H58.TriggerJob, G.TriggerJob>>() // ipc-triggers.ts TriggerJob
ok<Eq<H58.WaitingApproval, G.WaitingApproval>>() // ipc-triggers.ts WaitingApproval
ok<Eq<H58.DaemonStatus, G.DaemonStatus>>() // ipc-triggers.ts DaemonStatus
ok<Eq<H58.NotifySettings, G.NotifySettings>>() // ipc-triggers.ts NotifySettings
ok<Eq<H58.NotifyTest, G.NotifyTest>>() // ipc-triggers.ts NotifyTest
ok<Eq<H58.TriggersApi, G.TriggersApi>>() // ipc-triggers.ts TriggersApi
import type * as H59 from './ipc-visual.ts'
ok<Eq<H59.VisualTarget, G.VisualTarget>>() // ipc-visual.ts VisualTarget
ok<Eq<H59.VisualizeFormat, G.VisualizeFormat>>() // ipc-visual.ts VisualizeFormat
ok<Eq<H59.VisualResult, G.VisualResult>>() // ipc-visual.ts VisualResult
ok<Eq<H59.VisualizeCount, G.VisualizeCount>>() // ipc-visual.ts VisualizeCount
ok<Eq<H59.VisualizeResult, G.VisualizeResult>>() // ipc-visual.ts VisualizeResult
ok<Eq<H59.VoiceStatus, G.VoiceStatus>>() // ipc-visual.ts VoiceStatus
ok<Eq<H59.VisualApi, G.VisualApi>>() // ipc-visual.ts VisualApi
import type * as H60 from './ipc-web.ts'
ok<Eq<H60.WebArgs, G.WebArgs>>() // ipc-web.ts WebArgs
ok<Eq<H60.PlanRow, G.PlanRow>>() // ipc-web.ts PlanRow
ok<Eq<H60.VerifyResult, G.VerifyResult__web>>() // ipc-web.ts VerifyResult
ok<Eq<H60.RunReport, G.RunReport>>() // ipc-web.ts RunReport
ok<Eq<H60.HostItem, G.HostItem>>() // ipc-web.ts HostItem
ok<Eq<H60.SiteItem, G.SiteItem>>() // ipc-web.ts SiteItem
ok<Eq<H60.ClientItem, G.ClientItem>>() // ipc-web.ts ClientItem
ok<Eq<H60.WebApi, G.WebApi>>() // ipc-web.ts WebApi
import type * as H61 from './ipc-webview.ts'
ok<Eq<H61.WebviewApi, G.WebviewApi>>() // ipc-webview.ts WebviewApi
ok<Eq<H61.WindowEvents, G.WindowEvents>>() // ipc-webview.ts WindowEvents
import type * as H62 from './ipc.ts'
ok<Eq<H62.FileEntry, G.FileEntry>>() // ipc.ts FileEntry
ok<Eq<H62.ModelInfo, G.ModelInfo>>() // ipc.ts ModelInfo
ok<Eq<H62.SubagentEventInfo, G.SubagentEventInfo>>() // ipc.ts SubagentEventInfo
ok<Sub<H62.AgentEventBody, G.AgentEventBody>>() // ipc.ts AgentEventBody
ok<Sub<H62.AgentEvent, G.AgentEvent>>() // ipc.ts AgentEvent
ok<Sub<H62.LiveSession, G.LiveSession>>() // ipc.ts LiveSession
ok<Eq<H62.AppState, G.AppState>>() // ipc.ts AppState
ok<Eq<H62.UsageInfo, G.UsageInfo>>() // ipc.ts UsageInfo
ok<Eq<H62.SystemPromptInfo, G.SystemPromptInfo>>() // ipc.ts SystemPromptInfo
ok<Sub<H62.KorApi, Pick<G.KorIpcApi, keyof H62.KorApi>>>() // ipc.ts KorApi
ok<Eq<H62.SessionMeta, G.SessionMeta>>() // ipc.ts SessionMeta
ok<Eq<H62.TranscriptItem, G.TranscriptItem>>() // ipc.ts TranscriptItem
import type * as H63 from './g7-types.ts'
ok<Eq<H63.ThemeSetting, G.ThemeSetting>>() // g7-types.ts ThemeSetting
ok<Eq<H63.NotificationSound, G.NotificationSound>>() // g7-types.ts NotificationSound
ok<Eq<H63.NotificationPrefs, G.NotificationPrefs>>() // g7-types.ts NotificationPrefs
ok<Eq<H63.GitPrefs, G.GitPrefs>>() // g7-types.ts GitPrefs
ok<Eq<H63.ScriptPrefs, G.ScriptPrefs>>() // g7-types.ts ScriptPrefs
ok<Eq<H63.AgentProfile, G.AgentProfile>>() // g7-types.ts AgentProfile
ok<Eq<H63.EditorType, G.EditorType>>() // g7-types.ts EditorType
ok<Eq<H63.EditorPrefs, G.EditorPrefs>>() // g7-types.ts EditorPrefs
ok<Eq<H63.CatalogResponse, G.CatalogResponse>>() // g7-types.ts CatalogResponse
ok<Eq<H63.G7Settings, G.G7Settings>>() // g7-types.ts G7Settings
ok<Eq<H63.PermissionRulesInfo, G.PermissionRulesInfo>>() // g7-types.ts PermissionRulesInfo
ok<Eq<H63.TrustInfo, G.TrustInfo>>() // g7-types.ts TrustInfo
ok<Eq<H63.PrivacyFolder, G.PrivacyFolder>>() // g7-types.ts PrivacyFolder
ok<Eq<H63.PrivacyClearTarget, G.PrivacyClearTarget>>() // g7-types.ts PrivacyClearTarget
ok<Eq<H63.PrivacyInfo, G.PrivacyInfo>>() // g7-types.ts PrivacyInfo
ok<Eq<H63.CatalogModel, G.CatalogModel>>() // g7-types.ts CatalogModel
ok<Eq<H63.ProbeResult, G.ProbeResult__model_probe>>() // g7-types.ts ProbeResult
ok<Eq<H63.TaskModels, G.TaskModels>>() // g7-types.ts TaskModels
ok<Eq<H63.UsageHistory, G.UsageHistory>>() // g7-types.ts UsageHistory
ok<Eq<H63.UsagePeriod, G.UsagePeriod>>() // g7-types.ts UsagePeriod
