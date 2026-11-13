// GENERATED from backend/crates/kor-core/schema/ipc.json — do not edit (`node scripts/ipc-gen.ts`, spec §16.3, ADR 0061).
/* eslint-disable */

export interface AppState {
  /** Yeni sohbetlerin açılacağı klasör (açık sohbetin klasörü) */
  root: string | null
  model: string
  hasApiKey: boolean
  permissionMode: PermissionMode
  /** ~/.kor/settings.json içindeki varsayılan mod */
  defaultPermissionMode: PermissionMode
  /** Şu an çalışan sohbetler (arka planda olanlar dahil) */
  runningSessionIds: string[]
  /** Klasör seçicide gösterilen son kullanılan klasörler (en yeni önce) */
  recentWorkspaces: string[]
  /** Etkin arayüz dili (`auto` ise işletim sisteminden çözülmüş) */
  uiLanguage: 'tr' | 'en' | 'fr'
  /** Kullanıcının seçimi; CLI ile ortak `ui.language` ayarı */
  uiLanguagePref: 'auto' | 'tr' | 'en' | 'fr'
}

export type PermissionMode = 'auto' | 'default' | 'acceptEdits' | 'plan' | 'bypass'

export interface FileEntry {
  name: string
  /** Çalışma dizinine göre yol, '/' ayraçlı */
  path: string
  isDir: boolean
}

export interface ModelInfo {
  id: string
  name: string
  supportsTools: boolean
  /** Venice model_spec.uncensored */
  uncensored: boolean
}

export interface UsageInfo {
  /** Hesap bilgisi alınamazsa nedeni */
  accountError: string | null
  tier: string | null
  balances: { usd: number; diem: number; bundled: number } | null
  /** Günlük dönemin yenilendiği an (ISO) */
  nextEpochBegins: string | null
  keyExpiration: string | null
  /** Seçili modelin dakikalık sınırları */
  modelLimits: { rpm: number | null; tpm: number | null } | null
  /** Son yanıtın başlıklarından okunan anlık durum */
  live: { requests: RateLimitWindow | null; tokens: RateLimitWindow | null; observedAt: number } | null
  session: { promptTokens: number; completionTokens: number; costUsd: number | null }
}

export interface RateLimitWindow {
  limit: number
  remaining: number
  /** Epoch ms */
  resetAt: number
}

export interface SessionMeta {
  id: string
  title: string
  root: string
  model: string
  createdAt: number
  updatedAt: number
  pinned: boolean
  /** Kullanıcının verdiği ad; otomatik başlık bunu ezmez */
  customTitle?: string
  /** Modelin ürettiği kısa başlık */
  autoTitle?: string
  archived?: boolean
  /** Kullanıcı tanımlı bölüm (sections.json); bölüm silinmişse yok sayılır */
  sectionId?: string
  permissionMode?: PermissionMode
  /** Model kimliğine göre toplam token; maliyet arayüzde güncel fiyatlarla hesaplanır */
  usage?: Record<string, { promptTokens: number; completionTokens: number }>
  /** Son asistan yanıtının kısa özeti (kenar çubuğunda ikinci satır) */
  recap?: string
  /** [g26] Bu kayıt hangi sohbetten kopyalandı. */
  parentId?: string
  /** [cdx] Değişen dosya sayısı ve toplam +/- satır (kenar çubuğu satırı) */
  stats?: { files: number; added: number; removed: number }
}

export interface StoredSession {
  version: 1
  id: string
  title: string
  root: string
  model: string
  createdAt: number
  updatedAt: number
  pinned: boolean
  /** Kullanıcının verdiği ad; otomatik başlık bunu ezmez */
  customTitle?: string
  /** Modelin ürettiği kısa başlık */
  autoTitle?: string
  archived?: boolean
  /** Kullanıcı tanımlı bölüm (sections.json); bölüm silinmişse yok sayılır */
  sectionId?: string
  permissionMode?: PermissionMode
  /** Model kimliğine göre toplam token; maliyet arayüzde güncel fiyatlarla hesaplanır */
  usage?: Record<string, { promptTokens: number; completionTokens: number }>
  /** Son asistan yanıtının kısa özeti (kenar çubuğunda ikinci satır) */
  recap?: string
  /** [g26] Bu kayıt hangi sohbetten kopyalandı. */
  parentId?: string
  /** [cdx] Değişen dosya sayısı ve toplam +/- satır (kenar çubuğu satırı) */
  stats?: { files: number; added: number; removed: number }
  messages: ChatCompletionMessageParam[]
}

export type ChatCompletionMessageParam = import('openai/resources/chat/completions').ChatCompletionMessageParam

export type TranscriptItem = { kind: 'user'; text: string; attachments: string[] } | { kind: 'assistant'; text: string } | { kind: 'notice'; text: string } | { kind: 'tool'; id: string; name: string; summary: string; result: string; command?: string } | { kind: 'compaction'; summary: string }

/** Ana süreçte süren sohbet: yeniden yüklenen arayüz çalışma durumunu ve bekleyen kartları bundan kurar. */
export interface LiveSession {
  sessionId: string
  /** Tur başlangıcı (epoch ms); null: yalnızca arka plan alt ajanı sürüyor */
  since: number | null
  events: AgentEventBody[]
}

/** Her olay hangi sohbete ait olduğunu taşır: birden fazla sohbet aynı anda çalışabilir. */
export type AgentEventBody = { type: 'turnStart' } | { type: 'text'; delta: string } | { type: 'tool'; id: string; name: string; summary: string; parentId?: string } | { type: 'toolResult'; id: string; result: string } | { type: 'toolProgress'; id: string; line: string } | ({
  type: 'confirm'
  id: string
  name: string
  summary: string
  warning?: string
  preview?: { path: string; before: string | null; after: string }
} & ConfirmReviewFields) | {
  type: 'fileChanged'
  path: string
  before: string | null
  previous: string | null
  after: string
  toolId?: string
  /** [perf] büyük dosya: içerik gönderilmedi, istatistik ana süreçte hesaplandı; içerik getFileChange ile alınır */
  large?: { added: number; removed: number; bytes: number }
} | ({ type: 'subagent'; id: string } & SubagentEventInfo) | { type: 'subagentDone'; id: string; result: string; ok: boolean } | { type: 'notice'; text: string } | { type: 'estimate'; line: string; method: string; secs: number; usd: number | null; files: number } | { type: 'done' } | { type: 'error'; message: string } | { type: 'g2:queueConsumed'; items: { id: string; text: string; attachments: string[] }[] } | { type: 'g3:reasoning'; delta: string } | { type: 'g3:model'; from: string; to: string } | { type: 'g6:context'; info: ContextInfo } | ({ type: 'g6:compacted' } & CompactionEvent) | { type: 'g6:compactionState'; running: boolean } | { type: 'g6:compactionFailed'; message: string } | ({ type: 'g9:cost' } & TurnCostInfo) | { type: 'g10:event'; event: G10Event } | { type: 'g12:bulkDelete'; id: string; request: BulkDeleteRequest } | { type: 'g12:bulkDeleteDone'; id: string } | AgentStatusBody | QuestionEvent__body | CompactPreviewEvent__body | { type: 'paramCard'; id: string; card: ParamCard }

export interface SubagentEventInfo {
  agentType: string
  description: string
  background: boolean
  model: string
}

export interface ContextInfo {
  /** Bir sonraki istekte gidecek tahmini bağlam */
  usedTokens: number
  /** Modelin bağlam sınırı; model listesi alınamadıysa null */
  limitTokens: number | null
  /** Otomatik sıkıştırmanın tetiklendiği gerçek token sayısı (yüzde değil, tavan uygulanmış) */
  compactAtTokens: number | null
  autoCompactRatio: number
  promptTokens: number
  cachedTokens: number
  calls: number
  compactions: number
  /** Sıkıştırma sayesinde gönderilmeyen toplam token (brüt) */
  savedTokens: number
  /** Özetleme çağrılarının harcadığı token */
  compactionCostTokens: number
  /** Bağlam bütçesinin dağılımı (tahmin): yeni sohbet taslağında yok */
  breakdown?: ContextBreakdown
}

export interface ContextBreakdown {
  /** Sistem istemi (talimatlar, ortam, araç politikası) — bellek hariç */
  systemTokens: number
  /** Kurum/kullanıcı belleği, proje bilgisi ve oturum notları */
  memoryTokens: number
  /** Araç tanımları (JSON şema) */
  toolsTokens: number
  /** Konuşma geçmişi (kullanıcı, asistan, araç sonuçları) */
  historyTokens: number
}

export interface CompactionEvent {
  beforeTokens: number
  afterTokens: number
  auto: boolean
  summary: string
  /** Özeti üreten model (ayraçta gösterilir) */
  model: string
}

export interface TurnCostInfo {
  /** Bu görevin (tur + alt ajanları) harcaması, USD; akış sürerken tahmin içerir */
  turnUsd: number
  capUsd: number | null
  /** Akış sürüyor: değer çıktı uzunluğundan tahmin ediliyor */
  live: boolean
  /** Fiyatı bilinmeyen model kullanıldı: tutar eksik olabilir */
  unpriced: boolean
}

/** Arayüze (olay köprüsü) ve CLI'ya bildirilen güvenilirlik olayları. */
export type G10Event = { kind: 'circuitOpen'; target: string; retryInSec: number } | { kind: 'toolTimeout'; tool: string; seconds: number; reason: string } | { kind: 'reconnect'; attempt: number; continued: boolean } | { kind: 'degraded'; from: string; to: string; seconds: number } | { kind: 'contextTrimmed'; ok: boolean } | { kind: 'jsonRepaired'; tool: string; fixes: string[] } | { kind: 'refusal'; reason: string } | { kind: 'deadLetter'; title: string } | { kind: 'conflict'; path: string } | { kind: 'checkpoint'; files: number; partial: boolean }

export interface BulkDeleteRequest {
  command: string
  /** İlk dosyalar (en fazla 50), çalışma dizinine göre */
  files: string[]
  /** Bulunan dosya sayısı; capped ise gerçek sayı daha fazla olabilir */
  total: number
  capped: boolean
  /** Hedefi belirlenemeyen parçalar (değişken, komut ikamesi, find -delete, git clean...) */
  unresolved: string[]
}

export type Approval = 'yes' | 'no' | 'always'

export interface SystemPromptInfo {
  /** Kullanıcının kaydettiği istem; null ise varsayılan kullanılıyor */
  custom: string | null
  defaultInstructions: string
}

export interface Section {
  id: string
  name: string
  createdAt: number
}

export interface G2SendPayload {
  text: string
  /** Çalışma dizinine göre dosya veya klasör yolları */
  attachments: string[]
  images: G2ImageInput[]
  /** Çalışan turu iptal edip bu mesajla hemen yeni tur başlatır */
  interrupt: boolean
  reasoningEffort: ReasoningEffortLevel | null
}

export interface G2ImageInput {
  name: string
  /** data:image/...;base64,... — ana süreçte boyut ve sihirli baytla doğrulanır */
  dataUrl: string
}

export type ReasoningEffortLevel = 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max'

export type G2SendResult = { status: 'started' } | { status: 'queued'; queueId: string }

export interface G2FileHit {
  path: string
  isDir: boolean
}

export interface G2FileStat {
  path: string
  /** Klasörde veya okunamazsa null */
  size: number | null
  isDir: boolean
}

export interface G2DropResult {
  accepted: string[]
  rejected: { name: string; reason: string }[]
}

export interface G2ModelCaps {
  vision: boolean
  /** Modelin kabul ettiği ve arayüzün sunduğu düzeyler; boşsa seçici gizlenir */
  reasoningEfforts: ReasoningEffortLevel[]
  defaultReasoningEffort: string | null
}

export interface G2EditPayload {
  /** Kullanıcı mesajlarının arasındaki sırası (bildirimler hariç); bilinmiyorsa null */
  ordinal: number | null
  originalText: string
  newText: string
  reasoningEffort: ReasoningEffortLevel | null
}

export interface G2RegeneratePayload {
  /** Arayüzdeki son kullanıcı mesajının metni; geçmişle uyuşmazsa işlem reddedilir */
  text: string
  reasoningEffort: ReasoningEffortLevel | null
}

export type InteractAnswer = {
  kind: 'ask'
  selected: string[]
  other?: string
  answers?: { selected: string[]; other?: string; ranked?: string[]; skipped?: boolean }[]
} | { kind: 'planApprove'; mode: 'auto' | 'default' } | { kind: 'planRevise'; feedback: string } | { kind: 'draft'; confirmed: boolean }

export type OutputStyle = 'default' | 'explanatory' | 'concise' | 'learning'

export interface LinkPreview {
  url: string
  title: string | null
  description: string | null
  siteName: string | null
}

export interface SnippetResult {
  output: string
  exitCode: number | null
  timedOut: boolean
  truncated: boolean
}

export interface G4FileEntry extends FileEntry {
  /** Hassas dosya (.env, anahtarlar): önizleme, ekleme ve dosya işlemleri kapalı */
  sensitive: boolean
}

export interface FileListing {
  files: string[]
  /** Sınır aşıldı; liste eksik */
  truncated: boolean
}

export type FilePreview = {
  kind: 'text'
  content: string
  size: number
  mtimeMs: number
  /** Editörde açılabilir (hassas değil, boyut sınırı içinde) */
  editable: boolean
  /** SVG için <img> ile gösterilecek data URL (script çalışmaz) */
  imageDataUrl?: string
} | { kind: 'image'; dataUrl: string; size: number } | { kind: 'pdf'; pages: number | null; text: string; truncated: boolean; size: number } | { kind: 'binary'; size: number } | { kind: 'tooLarge'; size: number; limit: number } | { kind: 'sensitive' }

export type SaveResult = { ok: true; mtimeMs: number } | { ok: false; reason: 'conflict' }

export type EditorId = 'code' | 'cursor' | 'windsurf' | 'zed'

export type OpenedWith = EditorId | 'system' | 'folder'

export interface CheckpointInfo {
  id: string
  /** Kullanıcı mesajının session.messages içindeki konumu; konuşmayı bu noktadan kesmek için */
  messageIndex: number
  /** Kullanıcının yazdığı metin (ekler hariç); arayüz mesajla eşleştirmek için kullanır */
  text: string
  createdAt: number
  files: string[]
  /** false: bu nokta veya sonraki bir nokta bellek sınırını aştı, geri dönülemez */
  available: boolean
}

export interface RewindPreview {
  checkpoint: CheckpointInfo
  /** Yalnızca diskteki hali hedeften farklı olan dosyalar */
  files: { path: string; action: RewindAction; external: boolean }[]
  running: boolean
}

export type RewindAction = 'restore' | 'delete' | 'create'

export interface RewindResult {
  restored: string[]
  /** Oturumda değişen dosyaların güncel listesi (değişiklik paneli bununla yenilenir) */
  changes: RewindChange[]
  conversationCut: boolean
}

export interface RewindChange {
  path: string
  before: string | null
  /** Diskteki güncel hal; null = dosya yok */
  after: string | null
}

export interface DiagnosticsReport {
  tsc: ToolReport
  eslint: ToolReport
}

export interface ToolReport {
  tool: DiagnosticTool
  /** skipped: yapılandırma veya yerel kurulum yok; failed: çalıştırılamadı / çıktı okunamadı */
  status: 'ok' | 'skipped' | 'failed'
  reason: string | null
  diagnostics: Diagnostic[]
  truncated: boolean
  durationMs: number
}

export type DiagnosticTool = 'tsc' | 'eslint'

export interface Diagnostic {
  tool: DiagnosticTool
  /** Çalışma dizinine göre '/' ayraçlı yol; dizin dışındaysa veya genel hataysa ham metin */
  file: string
  /** false: tıklanınca açılamaz (dizin dışı veya dosyasız hata) */
  inWorkspace: boolean
  line: number
  column: number
  severity: 'error' | 'warning'
  code: string | null
  message: string
}

export interface CompactionResult {
  compacted: boolean
  /** compacted false ise nedeni */
  reason?: string
  beforeTokens: number
  afterTokens: number
}

export type InitProjectResult = { status: 'exists'; path: string } | { status: 'created'; path: string; backup: string | null; chars: number }

export interface ProjectMapInfo extends ProjectMapStatus {
  root: string | null
  /** Üretim sürüyorsa ilerleme */
  building: MapProgress | null
  /** Son üretim denemesi başarısızsa nedeni */
  error: string | null
  autoCreate: 'ask' | 'always' | 'never'
  /** AGENTS.md var mı (/init üzerine yazmadan önce onay ister) */
  agentsFileExists: boolean
}

export interface ProjectMapStatus {
  state: ProjectMapState
  generatedAt: number | null
  fileCount: number
  /** Yeni, değişen veya silinen alanların etiketleri */
  changedAreas: string[]
  /** Haritayı baştan üretmenin token maliyeti: her yeni sohbette yeniden keşif yerine kaçınılan miktarın tahmini */
  buildTokens: number
  truncated: boolean
}

export type ProjectMapState = 'none' | 'fresh' | 'stale'

export interface MapProgress {
  /** init: /init için AGENTS.md taslağı yazılıyor */
  phase: 'scan' | 'explore' | 'synthesize' | 'write' | 'init'
  done: number
  total: number
  /** Şu an keşfedilen alanlar */
  current: string[]
}

export interface ContextView extends ContextInfo {
  /** Oturumun sistem isteminde harita varsa: haritayı baştan üretmenin token maliyeti (yeniden keşiften kaçınılan tahmini miktar) */
  mapSavedTokens: number | null
}

export interface CompactSettings {
  /** Otomatik sıkıştırma eşiği tavanı (token); null = varsayılan */
  maxTokens: number | null
  /** Eşik oranı (0.3-0.95); null = varsayılan */
  ratio: number | null
}

export interface CompactSettingsPatch {
  maxTokens?: number | null
  ratio?: number | null
}

export interface ProvidersView {
  providers: ProviderView[]
  /** Anahtarlar işletim sisteminin anahtarlığıyla şifrelenip diske yazılıyor mu */
  persisted: boolean
  /** Venice anahtarı ortam değişkeninden geliyor (profil yok) */
  envVenice: boolean
  settings: G9Settings
  responseCache: { enabled: boolean; entries: number }
}

export interface ProviderView {
  id: ProviderId
  label: string
  defaultBaseURL: string
  baseURL: string
  keyless: boolean
  uncensored: boolean
  usable: boolean
  profiles: KeyProfileView[]
  endpoint: ProviderEndpoint
  /** Son model listesi hatası */
  catalogError: string | null
}

export type ProviderId = 'venice' | 'veniceweb' | 'openai' | 'anthropic' | 'openrouter' | 'groq' | 'experiential' | 'uplinkswitch' | 'abliteration' | 'featherless' | 'arliai' | 'infermatic' | 'deepinfra' | 'fireworks' | 'together' | 'huggingface' | 'ollama'

/** Anahtarın kendisi arayüze hiç gelmez; yalnızca son 4 karakteri. */
export interface KeyProfileView {
  id: string
  label: string
  hint: string
  backup: boolean
  active: boolean
  createdAt: number
}

export interface ProviderEndpoint {
  baseURL?: string
  headers?: Record<string, string>
  timeoutMs?: number
  maxConcurrent?: number
}

export interface G9Settings {
  endpoints?: Record<string, {
    baseURL?: string
    headers?: Record<string, string>
    timeoutMs?: number
    maxConcurrent?: number
  }>
  ollamaEnabled?: boolean
  presets?: Record<string, { temperature?: number; top_p?: number; maxTokens?: number }>
  modelPresets?: Record<string, string>
  prices?: Record<string, { input: number; output: number; cacheInput?: number; contextTokens?: number }>
  costCapUsd?: number | null
  lowCostMode?: boolean
  responseCache?: { enabled: boolean; ttlHours: number; exclude: string[] }
}

export interface ProbeResult {
  provider: ProviderId
  ok: boolean
  status: number | null
  latencyMs: number | null
  message: string
}

export interface G9SettingsPatch {
  ollamaEnabled?: boolean
  presets?: Record<string, { temperature?: number; top_p?: number; maxTokens?: number }>
  modelPresets?: Record<string, string>
  prices?: Record<string, { input: number; output: number; cacheInput?: number; contextTokens?: number }>
  costCapUsd?: number | null
  lowCostMode?: boolean
  responseCache?: { enabled: boolean; ttlHours: number; exclude: string[] }
}

export interface CostInfo {
  /** Seçili modelin fiyatı (USD / 1M token); bilinmiyorsa null */
  price: ModelPrice | null
  /** Bir sonraki istekte gidecek tahmini bağlam */
  contextTokens: number
  /** Oturum boyunca gönderilen ve önbellekten gelen girdi token'ı */
  promptTokens: number
  cachedTokens: number
  sessionUsd: number
  unpriced: boolean
  capUsd: number | null
}

export interface ModelPrice {
  /** USD / 1M token */
  input: number
  output: number
  cacheInput: number | null
  cacheWrite: number | null
}

export interface OllamaView {
  models: OllamaModel[]
  running: OllamaRunning[]
  pulls: PullState[]
}

export interface OllamaModel {
  name: string
  sizeBytes: number
  family: string | null
  parameterSize: string | null
  quantization: string | null
}

export interface OllamaRunning {
  name: string
  sizeBytes: number
  vramBytes: number
  expiresAt: string | null
}

export interface PullState extends PullProgress {
  name: string
  error: string | null
  done: boolean
}

export interface PullProgress {
  status: string
  completed: number | null
  total: number | null
}

export interface VeniceWebView {
  /** Kayıtlı veya bekleyen giriş var mı */
  hasSession: boolean
  /** Oturum kullanılabilir mi (JWT tazelenebiliyor) */
  ready: boolean
  email: string | null
  savedAt: number | null
  /** 2FA bekleniyor mu (kod formu gösterilir) */
  pendingSecondFactor: boolean
  /** Kodun nereden geldiğini anlatan kısa etiket */
  pendingHint: string | null
  /** Kodun gönderildiği maskeli adres */
  pendingTarget: string | null
  /** Taşıma: wreq (Chrome parmak izi) veya fetch */
  transport: string
  /** Giriş formunu önceden doldurmak için son kullanılan e-posta */
  lastEmail: string | null
  lastError: string | null
}

export interface VeniceWebLoginResult {
  status: 'signed_in' | 'needs_second_factor'
  email: string
  /** 2FA gerektiğinde kodun gittiği maskeli adres */
  safeIdentifier: string | null
  strategy: string | null
  transport: string
}

export interface Trigger {
  id: string
  name: string
  prompt: string
  intervalMinutes: number
  root: string
  enabled: boolean
  createdAt: number
  lastRun: number | null
  nextRun: number
}

export interface RoutineInput {
  name?: string
  prompt: string
  intervalMinutes: number
  root: string
}

export interface TerminalRunResult {
  output: string
  code: number | null
  cwd: string
}

export interface GitStatus {
  isRepo: boolean
  /** Depo klasör adı (kök) */
  repoName: string
  branch: string | null
  /** Takip edilen uzak dala göre ileri/geri commit sayısı; upstream yoksa null */
  ahead: number | null
  behind: number | null
  /** `git status --porcelain` satır sayısı (kaydedilmemiş değişiklik) */
  dirty: number
  remoteUrl: string | null
  /** GitHub web adresi (https://github.com/owner/repo) çıkarılabildiyse */
  webUrl: string | null
  /** Bu dal için açık PR (gh CLI varsa) */
  pr: { number: number; url: string } | null
  /** gh CLI bulundu mu (PR oluşturma gh ile; yoksa tarayıcıda compare sayfası açılır) */
  hasGh: boolean
  /** Süren rebase/merge (cdesktop "Rebase in progress" şeridi) */
  operation: 'rebase' | 'merge' | null
  /** Çakışmalı dosyalar (git diff --name-only --diff-filter=U) */
  conflicts: string[]
  /** Varsayılan taban dal (origin/HEAD → main/master) */
  defaultBase: string | null
}

export interface GitBranches {
  current: string | null
  /** Varsayılan taban dal (origin/HEAD → main/master) */
  defaultBase: string | null
  local: string[]
}

export interface CreatePrResult {
  /** gh ile oluşturulduysa PR adresi; tarayıcıda compare sayfası açıldıysa o adres */
  url: string
  /** true: gh ile gerçekten oluşturuldu; false: tarayıcıda form açıldı */
  created: boolean
}

export interface CreatePrInput {
  title: string
  body: string
  base: string
  draft: boolean
}

export interface DevServerStatus {
  running: boolean
  /** Ayarlardaki dev betiği (yoksa null) */
  command: string | null
  /** Çıktıda yakalanan yerel URL (yoksa null) */
  url: string | null
  pid: number | null
  exitCode: number | null
  /** Son satırlar (en çok 200) */
  log: string[]
}

export interface PreviewBounds {
  x: number
  y: number
  width: number
  height: number
}

export interface BrowserBounds {
  x: number
  y: number
  width: number
  height: number
}

export interface BrowserTabInfo {
  id: string
  url: string
  title: string
  loading: boolean
  active: boolean
  canGoBack: boolean
}

export interface LogEntry {
  /** ISO zaman damgası */
  ts: string
  tool: string
  decision: 'allow' | 'deny' | 'approved' | 'rejected' | 'invalid'
  ok?: boolean
  durationMs?: number
  reason?: string
  /** Kısaltılmış argüman özeti (JSON) */
  args: string
}

export interface Tag {
  /** @ ile kullanılan ad: harf/rakam ile başlar; harf, rakam, _ ve - (en fazla 64) */
  name: string
  content: string
}

export interface McpServerView {
  config: McpServerConfig
  status: McpServerStatus
}

export type McpServerConfig = {
  transport: 'stdio'
  id: string
  label?: string
  enabled: boolean
  command: string
  args: string[]
  env: Record<string, string>
} | {
  transport: 'http'
  id: string
  label?: string
  enabled: boolean
  url: string
  headers: Record<string, string>
}

export interface McpServerStatus {
  id: string
  label: string | null
  transport: 'stdio' | 'http'
  /** Gösterim için komut+argüman ya da URL (gizli env/başlık değerleri burada yer almaz). */
  target: string
  enabled: boolean
  state: 'connected' | 'error' | 'disabled'
  toolCount: number
  error: string | null
}

export interface ConnectionsView {
  ssh: PublicSshHost[]
  databases: PublicDbConnection[]
  /** false: işletim sistemi kasası yok; parola kaydedilemez (anahtar/agent kullanılmalı) */
  canStoreSecrets: boolean
}

/** Arayüze ve modele dönen biçim: şifreli sır yerine yalnızca "kayıtlı mı". */
export type PublicSshHost = {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth: 'password' | 'key' | 'agent'
  keyPath?: string
  hostKey?: string
} & { hasSecret: boolean }

export interface SshHost {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth: 'password' | 'key' | 'agent'
  keyPath?: string
  secret?: string
  hostKey?: string
}

export type PublicDbConnection = {
  id: string
  name: string
  kind: 'postgres' | 'mysql' | 'sqlite'
  host?: string
  port?: number
  database?: string
  user?: string
  file?: string
  ssl?: boolean
  sshHost?: string
} & { hasSecret: boolean }

export interface DbConnection {
  id: string
  name: string
  kind: 'postgres' | 'mysql' | 'sqlite'
  host?: string
  port?: number
  database?: string
  user?: string
  file?: string
  ssl?: boolean
  sshHost?: string
  secret?: string
}

export type SshHostInput = {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth: 'password' | 'key' | 'agent'
  keyPath?: string
} & SecretInput

export interface SecretInput {
  password?: string
  clearSecret?: boolean
}

export type DbConnectionInput = {
  id: string
  name: string
  kind: 'postgres' | 'mysql' | 'sqlite'
  host?: string
  port?: number
  database?: string
  user?: string
  file?: string
  ssl?: boolean
  sshHost?: string
} & SecretInput

export type ConnectionTestResult = { ok: true } | { ok: false; error: string } | { ok: false; hostKey: { fingerprint: string; mismatch: boolean } }

export interface FileCommand {
  name: string
  description: string
  argumentHint: string | null
  body: string
  source: 'user' | 'project' | 'plugin'
}

export interface TaskTemplate {
  name: string
  description: string
  source: 'project' | 'personal' | 'plugin' | 'builtin'
  path: string | null
  model: string | null
  inputs: TemplateInput[]
  steps: string[]
  acceptance: string[]
}

export interface TemplateInput {
  name: string
  kind: TemplateInputKind
  required: boolean
  default: string | null
  description: string | null
}

export type TemplateInputKind = { type: 'string' } | { type: 'number' } | { type: 'bool' } | { type: 'path' } | { type: 'enum'; values: string[] }

export interface RenderedTemplate {
  /** Ajana gönderilecek istem (gövde + sıralı adımlar + kabul ölçütleri). */
  prompt: string
  model: string | null
}

export interface AgentSummary {
  name: string
  description: string
  source: 'builtin' | 'embedded' | 'user' | 'project'
  /** Eklentiden geliyorsa eklenti adı */
  origin: string | null
  model: string | null
  /** 'all' = tüm araçlar; aksi halde izin verilen araç sayısı. */
  tools: 'all' | number
  manualApproval: boolean
}

export interface TeamView {
  name: string
  description: string
  roles: TeamRoleView[]
  source: 'builtin' | 'user' | 'project'
}

export interface TeamRoleView {
  role: string
  agent: string
  instructions: string
}

/** Kor tarayıcı eklentisi köprüsünün durumu (ADR 0058). */
export interface ChromeBridgeInfo {
  /** Köprü bu süreçte çalışıyor mu (127.0.0.1:49201). */
  running: boolean
  /** Eşleşmiş bir tarayıcı şu an bağlı mı. */
  connected: boolean
  /** Bağlı eşleşmiş tarayıcı sayısı. */
  connections: number
  /** Köprü portu (yalnızca 127.0.0.1). */
  port: number
  /** Anahtar zincirinde kayıtlı eşleşmiş tarayıcı sayısı. */
  paired: number
  /** Girilmeyi bekleyen bir eşleştirme kodu var mı. */
  codeWaiting: boolean
  /** Tarayıcı başına yerel mesajlaşma ana bilgisayarı kayıtlı mı. */
  hosts: Record<string, boolean>
  domains: ChromeDomains
  /** browser.backend: auto, chrome veya headless. */
  backend: string
  /** Chrome/Edge/Brave derlemesinin klasörü (Paketlenmemiş yükle). */
  extensionPath: string
  /** Firefox derlemesinin klasörü. */
  firefoxPath: string
  /** Paketlenmemiş derlemenin sabit kimliği. */
  extensionId: string
}

export interface ConfigExplain {
  paths: { system: string | null; user: string | null; project: string | null }
  settings: {
    key: string
    value: string
    layer: 'default' | 'system' | 'user' | 'project' | 'env' | 'cli'
    origin: string
  }[]
}

export interface UndoJournal {
  session: string
  paths: number
  modifiedAt: number | null
}

export interface UndoReport {
  restored: string[]
  deleted: string[]
  skipped: string[]
  failed: { path: string; error: string }[]
}

export interface CompatStatus {
  trusted: boolean
  instructions: { source: CoreSource; applies: unknown; chars: number }[]
  permissions: {
    allow: { rule: string; source: CoreSource }[]
    ask: { rule: string; source: CoreSource }[]
    deny: { rule: string; source: CoreSource }[]
    mode: { mode: string; source: CoreSource } | null
  }
  hooks: { event: string; matcher: string | null; command: string; source: CoreSource }[]
  commands: { name: string; source: CoreSource }[]
  agents: { name: string; source: CoreSource }[]
  skills: { name: string; source: CoreSource }[]
  ignoreFiles: string[]
  untrustedSkipped: string[]
  warnings: string[]
}

export interface CoreSource {
  eco: 'kor' | 'claude' | 'codex' | 'cursor'
  scope: 'user' | 'project' | 'local'
  path: string
}

export interface Marketplace {
  name: string
  path: string
  plugins: string[]
}

export interface DiscoveredPlugin {
  name: string
  /** Klon/klasör adı (manifest adından farklı olabilir; güncelle/kaldır bununla yapılır). */
  dirName: string
  dir: string
  source: 'user' | 'project'
  manifest: PluginManifest
  enabled: boolean
  contributions: PluginContributions
  /** .git içeriyor: git kurucusuyla kurulmuş (güncellenebilir/kaldırılabilir). */
  managed: boolean
}

export interface PluginManifest {
  name: string
  version: string | null
  description: string
  author: string | null
  /** plugin.json 'skills' alanı (yol ya da yol listesi) */
  skills?: string | string[]
}

export interface PluginContributions {
  commands: number
  skills: number
  agents: number
  tools: number
}

export type CatalogItem = CatalogEntry & { installed: boolean }

export type CatalogEntry = CatalogGitPlugin | CatalogMcpServer

export interface CatalogGitPlugin {
  kind: 'plugin'
  /** Klon klasörü adı (NAME_RE'ye uymalı). */
  name: string
  repo: string
  description: string
  author: string
}

export interface CatalogMcpServer {
  kind: 'mcp'
  id: string
  label: string
  /** npx ile çalıştırılan paket (rea-agents@sürüm); platforma göre komut işleyicide kurulur. */
  npmPackage: string
  mcpArgs: string[]
  description: string
}

export interface StoreItem extends StorePlugin {
  installed: boolean
}

export interface StorePlugin {
  /** Türetilmiş klon klasörü adı (NAME_RE güvenli). */
  name: string
  /** owner/repo */
  fullName: string
  description: string
  author: string
  /** https github.com klon adresi. */
  repoUrl: string
  stars: number
  topics: string[]
}

/** Yarıda kalmış ve kurtarılabilir bir tur */
export interface RecoverableTurn {
  sessionId: string
  title: string
  root: string
  startedAt: number
  /** Başarıyla tamamlanmış araç adımları */
  steps: number
  failedSteps: number
  /** Turda değiştirilen dosyalar (yazma araçları) */
  files: string[]
  /** true: uygulama kapanırken/çökerken kaldı; false: tur hata verdi */
  crashed: boolean
}

export interface RestoreResult {
  /** Tamamlanmış adımlar eklenmiş konuşma */
  transcript: TranscriptItem[]
  /** Devam turu için gönderilecek mesaj */
  prompt: string
}

export interface DeadLetter {
  id: string
  sessionId: string
  kind: 'subagent' | 'shell'
  title: string
  error: string
  at: number
}

export interface CheckItem {
  id: 'apiKey' | 'network' | 'disk' | 'git' | 'ripgrep' | 'node' | 'config'
  label: string
  status: CheckStatus
  message: string
}

export type CheckStatus = 'ok' | 'warn' | 'fail'

export interface PendingMemory {
  id: string
  text: string
  createdAt: number
}

/** Arayüz özeti */
export interface MemoryOverview {
  session: MemoryRecord[]
  project: { root: string; trusted: boolean; records: MemoryRecord[]; error?: string } | null
  global: MemoryRecord[]
  conflicts: Conflict[]
  confidenceScore: boolean
}

export interface MemoryRecord {
  id: string
  layer: MemoryLayer
  text: string
  kind?: KnowledgeKind
  subject?: string
}

export type MemoryLayer = 'session' | 'project' | 'global'

export type KnowledgeKind = 'decision' | 'constraint' | 'term' | 'note'

export interface Conflict {
  a: string
  b: string
  reason: string
}

export interface SandboxStatus {
  /** Kullanıcı ayar dosyasında "sandbox": true */
  enabled: boolean
  /** Bu platformda gerçek bir çekirdek izolasyonu kurulabiliyor */
  available: boolean
  mechanism: 'bwrap' | null
  /** Kullanıcıya gösterilecek açıklama */
  message: string
}

export interface G12ToolScope {
  name: string
  kind: ToolKind
  level: G12ToolLevel
}

/** read: yan etkisiz; edit: dosya değiştirir; exec: komut çalıştırır */
export type ToolKind = 'read' | 'edit' | 'exec'

export type G12ToolLevel = 'default' | 'ask' | 'allow' | 'deny'

export interface PromptLayers {
  /** Sohbetin klasörü (sohbet bellekte değilse açık klasör) */
  root: string
  global: { custom: boolean; chars: number }
  project: { file: string; text: string; trusted: boolean }
  session: { text: string }
  /** Bir sonraki model çağrısında gönderilecek birleşik sistem istemi */
  preview: string
}

export interface EnvKey {
  key: string
  empty: boolean
}

export interface EnvTemplateReport {
  /** Bulunan şablon dosyası; yoksa null (denetim yapılmadı) */
  template: string | null
  /** Şablonda var, .env/.env.local'da hiç yok */
  missing: string[]
  /** Var ama değeri boş */
  empty: string[]
  /** .env'de var, şablonda yok (şablon güncel değil olabilir) */
  extra: string[]
}

export type ExportFormat = 'markdown' | 'json'

export interface Overview {
  now: number
  health: HealthInfo
  tools: ToolStat[]
  models: ModelStat[]
  errors: ErrorBucket[]
  slow: SlowCall[]
  /** En yeni önce */
  turns: TurnRecord[]
  ratings: Record<string, 1 | -1>
  /** En yeni önce */
  events: TelemetryEvent[]
  live: (LiveTurn & { elapsedMs: number })[]
  settings: ObservabilitySettings
  exportStatus: ExportStatus
}

export interface HealthInfo {
  /** İşletim sisteminin ağ bağlantısı bildirimi (istek atmadan) */
  online: boolean
  provider: {
    state: ProviderState
    host: string
    lastCallAt: number | null
    lastStatus: SpanStatus | null
    /** Son 10 dakikadaki model çağrılarında hata oranı (0-1) */
    errorRate: number
    recentCalls: number
    p95Ms: number | null
  }
  /** Son etkin erişilebilirlik denetimi (yalnızca istek üzerine) */
  probe: { at: number; ok: boolean; latencyMs: number; status: number | null; error: string | null } | null
  memoryMb: number
  memoryBudgetMb: number
}

export type ProviderState = 'ok' | 'degraded' | 'down' | 'unknown'

export type SpanStatus = 'ok' | 'error' | 'cancelled' | 'denied'

export interface ToolStat extends LatencySummary {
  name: string
  ok: number
  errors: number
  denied: number
  cancelled: number
  slow: number
  /** 0-1; reddedilen/iptal edilen çağrılar paydada yer almaz */
  successRate: number
}

export interface LatencySummary {
  count: number
  p50: number
  p95: number
  max: number
  sum: number
}

export interface ModelStat extends LatencySummary {
  model: string
  errors: number
  /** İlk token süresi (ms) medyanı; akış görülmediyse null */
  ttftP50: number | null
  promptTokens: number
  completionTokens: number
  cachedTokens: number
}

export interface ErrorBucket {
  start: number
  toolCalls: number
  toolErrors: number
  modelCalls: number
  modelErrors: number
}

export interface SlowCall extends ToolCallRecord {
  hint: string
}

export interface ToolCallRecord {
  id: string
  traceId: string
  sessionId: string
  name: string
  /** Maskeli, kısaltılmış özet (dosya yolu, komut) */
  summary: string
  start: number
  end: number
  status: SpanStatus
  slow: boolean
}

export interface TurnRecord {
  traceId: string
  spanId: string
  parentSpanId: string | null
  sessionId: string
  /** Alt ajan turuysa ajan tipi */
  subagent: string | null
  project: string
  model: string
  start: number
  end: number
  status: SpanStatus
  message?: string
  steps: TimelineStep[]
  tokens: TokenSplit & { completion: number; cached: number }
  costUsd: number | null
  modelCalls: number
  toolCalls: number
}

export interface TimelineStep {
  kind: StepKind
  label: string
  start: number
  end: number
  status: SpanStatus
}

export type StepKind = 'wait' | 'thinking' | 'response' | 'tool'

/** İstem token'larının kaynağa göre kırılımı (tahmin, gerçek prompt_tokens'a ölçeklenmiş). */
export interface TokenSplit {
  system: number
  toolDefs: number
  toolResults: number
  history: number
}

/** Olay akışı satırı (kronolojik, filtrelenebilir). */
export interface TelemetryEvent {
  id: number
  at: number
  sessionId: string
  traceId: string
  kind: TelemetryEventKind
  phase: 'start' | 'end' | 'info'
  label: string
  status?: SpanStatus
  durationMs?: number
}

export type TelemetryEventKind = 'turn' | 'model' | 'tool' | 'subagent' | 'alert' | 'compact'

export interface LiveTurn {
  sessionId: string
  traceId: string
  start: number
  subagent: string | null
}

/** ~/.kor/settings.json "observability" bölümü; yalnızca kullanıcı dosyasından okunur. */
export interface ObservabilitySettings {
  /** Tamamlanan tur izlerini OTLP/JSON olarak yerel dosyaya yaz */
  traceExport: boolean
  /** Boş: kapalı. Yalnızca http(s)://127.0.0.1 | localhost | [::1] (ör. http://127.0.0.1:4318/v1/traces) */
  otlpEndpoint: string
  /** Prometheus /metrics uç noktası (yalnızca 127.0.0.1) */
  prometheus: boolean
  prometheusPort: number
  slowToolMs: number
  turnBudgetMs: number
  memoryBudgetMb: number
  /** Yavaş araç ve bütçe uyarıları bildirim olarak gösterilsin */
  alerts: boolean
}

export interface ExportStatus {
  traceDir: string
  traceError: string | null
  prometheusUrl: string | null
  prometheusError: string | null
  otlpSent: number
  otlpError: string | null
}

export interface CostReport {
  days: number
  attribution: CostAttribution
  /** Yerel defterden (bu uygulamanın çağrıları) son 30 güne göre */
  forecast: SpendForecast
  recommendation: ModelRecommendation
  tags: Record<string, string[]>
}

export interface CostAttribution {
  totalUsd: number
  /** Fiyatı bilinmeyen modellerin çağrı sayısı (maliyet 0 sayıldı) */
  unpricedCalls: number
  byModel: CostSlice[]
  byTool: CostSlice[]
  byProject: CostSlice[]
  byTag: CostSlice[]
  byDay: CostSlice[]
}

export interface CostSlice {
  key: string
  usd: number
  tokens: number
  calls: number
}

export interface SpendForecast {
  /** Son 7 günün (bugün dahil) günlük ortalaması */
  avg7: number
  avg30: number
  nextWeekUsd: number
  monthToDateUsd: number
  /** Ay sonu tahmini: ay başından bugüne gerçekleşen + kalan günler x avg7 */
  monthProjectionUsd: number
  daysLeftInMonth: number
  /** Veri olan gün sayısı; azsa tahmin güvenilmez */
  sampleDays: number
}

export interface ModelRecommendation {
  stats: ModelQuality[]
  pick: string | null
  reason: string
}

export interface ModelQuality {
  model: string
  likes: number
  dislikes: number
  /** Laplace düzeltmeli beğeni oranı: (beğeni + 1) / (toplam + 2) */
  score: number
  /** Değerlendirilen turların ortalama maliyeti; fiyat bilinmiyorsa null */
  avgTurnCostUsd: number | null
}

/** Ham istek/yanıt denetçisi kaydı; metinler maskeli ve kısaltılmış JSON. */
export interface RawExchange {
  id: string
  at: number
  sessionId: string
  model: string
  status: SpanStatus
  durationMs: number
  request: string
  response: string
}

export interface ProcessReport {
  at: number
  /** Electron'un kendi süreçleri (ana, arayüz, GPU) */
  app: { pid: number; type: string; cpuPercent: number; memMb: number }[]
  groups: ProcessGroup[]
  shells: { id: string; command: string; sessionId: string; startedAt: number }[]
  error: string | null
}

export interface ProcessGroup {
  /** Arka plan kabuğu kimliği (ShellRegistry) eşleştiyse */
  shellId: string | null
  label: string
  rootPid: number
  pids: number[]
  memBytes: number
  /** İlk örnekte (karşılaştırılacak önceki ölçüm yok) null */
  cpuPercent: number | null
}

export interface ComposerStatus {
  online: boolean
  provider: ProviderState
  lastTurn: { traceId: string; model: string; rating: 1 | -1 | null } | null
}

export interface UpdateStatus {
  version: string
  distribution: Distribution
  /** Güncelleme denetimi bu kurulumda kullanılabilir mi */
  enabled: boolean
  /** Kapalıysa nedeni (yapılandırma yok, portable, MSI, kurumsal politika...) */
  reason: string | null
  source: 'kurumsal' | 'kullanıcı' | null
  /** Ana makine adı; tam URL arayüze gönderilmez */
  host: string | null
  phase: UpdatePhase
  /** Bulunan sürüm */
  available: string | null
  /** Bulunan sürümün sayfası (masaüstü kurulum paketiyle güncellenir) */
  releaseUrl?: string | null
  /** İndirme yüzdesi (0-100) */
  percent: number | null
  error: string | null
  /** update.channel (stable/beta/nightly) */
  channel?: string
  /** Bu derleme imzalı paketi yerinde kurabilir (yoksa sürüm sayfası açılır) */
  signed?: boolean
}

/** Tauri paket türü (nsis/msi/mac/appimage/deb/rpm); dev: paketlenmemiş çalışma. portable eski sürümlerden kalır. */
export type Distribution = 'nsis' | 'msi' | 'portable' | 'mac' | 'appimage' | 'deb' | 'rpm' | 'dev'

export type UpdatePhase = 'idle' | 'checking' | 'available' | 'not-available' | 'downloading' | 'downloaded' | 'error'

export interface CrashInfo {
  events: CrashEvent[]
  dumps: DumpInfo[]
  /** settings.json'da crashReports.uploadUrl tanımlı mı (gönder düğmesi yalnızca o zaman görünür) */
  uploadHost: string | null
}

export interface CrashEvent {
  at: number
  process: string
  reason: string
  exitCode: number | null
  detail?: string
}

export interface DumpInfo {
  name: string
  size: number
  mtimeMs: number
}

export interface ConfigImportResult extends ImportSummary {
  systemPrompt: boolean
  outputStyle: boolean
}

export interface ImportSummary {
  /** Yazılacak ayar anahtarları (ör. model, theme, permissions.deny) */
  applied: string[]
  /** Dosyada olup güvenlik nedeniyle yok sayılanlar */
  skipped: string[]
}

export interface G20PythonResult {
  chunks: G20Chunk[]
  exitCode: number | null
  timedOut: boolean
  truncated: boolean
  durationMs: number
  /** Kullanılan yorumlayıcı (proje sanal ortamı ya da sistem Python'u) */
  interpreter: string
  images: G20Image[]
  vars: G20VarInfo[]
}

export interface G20Chunk {
  stream: 'out' | 'err'
  text: string
}

export interface G20Image {
  name: string
  /** data:image/...;base64,... (ana belge CSP'si img-src data: izinli; <img> içinde betik çalışmaz) */
  dataUrl: string
}

export interface G20VarInfo {
  name: string
  type: string
  preview: string
}

export interface G20SqlResult {
  columns: string[]
  rows: string[][]
  truncated: boolean
  durationMs: number
}

export interface G22ModelSupport {
  model: string
  caps: ModelCapabilities
  /** Kullanıcının bu model için seçtiği araç modu (varsayılan otomatik) */
  toolMode: ToolMode
  files: G22FileSupport[]
}

export interface ModelCapabilities {
  /** 'unknown': yerel araç çağrısı denenir; API reddederse oturum metin protokolüne düşer (text-tools-g22.ts) */
  tools: boolean | 'unknown'
  vision: boolean
  /** Model PDF'yi kendisi okuyabiliyorsa (dosya parçası). false: PDF yerelde metne dönüştürülür. */
  pdf: boolean
  audio: boolean
  contextTokens: number | null
  /** response_format json_schema desteği (structured-output.ts); bilinmiyorsa şema isteme yazılır. */
  responseSchema?: boolean
}

export type ToolMode = 'auto' | 'native' | 'text' | 'off'

export type UserSettingsFile = SettingsFile

export interface SettingsFile {
  model?: string
  maxTurns?: number
  shell?: 'cmd' | 'powershell' | 'pwsh' | 'git-bash'
  additionalDirectories?: string[]
  autoVerify?: boolean
  compactModel?: string
  compactAtTokens?: number | null
  compactRatio?: number | null
  projectMapAutoCreate?: 'ask' | 'always' | 'never'
  g9?: Record<string, unknown>
  g10?: { toolTimeoutSec?: number; firstTokenTimeoutSec?: number; fallbackModel?: string }
  confidenceScore?: boolean
  sandbox?: boolean
  docSources?: string[]
  observability?: {
    traceExport?: boolean
    otlpEndpoint?: string
    prometheus?: boolean
    prometheusPort?: number
    slowToolMs?: number
    turnBudgetMs?: number
    memoryBudgetMb?: number
    alerts?: boolean
  }
  updates?: { url?: string; autoCheck?: boolean }
  crashReports?: { uploadUrl?: string }
  toolModes?: Record<string, 'auto' | 'native' | 'text' | 'off'>
  modelAliases?: Record<string, string>
  taskModels?: { explore?: string; plan?: string; review?: string; title?: string; compaction?: string }
  theme?: 'system' | 'light' | 'dark'
  notifications?: { desktop?: boolean; sound?: boolean; soundName?: 'soft' | 'double' | 'bell' | 'chime' }
  git?: { branchPrefix?: string; worktreeDir?: string }
  scripts?: { setup?: string; cleanup?: string; dev?: string }
  profiles?: {
    name: string
    mode?: 'auto' | 'default' | 'acceptEdits' | 'plan' | 'bypass'
    model?: string
    effort?: 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max'
  }[]
  logs?: { keepDays?: number }
  editor?: {
    type?: 'code' | 'cursor' | 'windsurf' | 'zed' | 'custom' | 'system'
    customCommand?: string
    sshHost?: string
  }
  permissions?: {
    mode?: 'auto' | 'default' | 'acceptEdits' | 'plan' | 'bypass'
    allow?: string[]
    deny?: string[]
    autoMemory?: boolean
  }
}

export interface G22FileSupport {
  path: string
  kind: AttachmentKind
  plan: AttachmentPlan
}

export type AttachmentKind = 'image' | 'pdf' | 'docx' | 'xlsx' | 'audio' | 'opk' | 'other'

/** native: modele dosya/ses parçası olarak; text: metne dönüştürülerek (other: olduğu gibi metin); unsupported: gönderilemez */
export type AttachmentPlan = 'native' | 'text' | 'unsupported'

export interface CatalogResponse {
  models: CatalogModel[]
  /** Takma ad → model kimliği (tüm ayar katmanlarından) */
  aliases: Record<string, string>
  aa: {
    status: 'ok' | 'stale' | 'error'
    fetchedAt: number | null
    message: string | null
    /** AA puanı eşleşen Venice modeli sayısı */
    matched: number
  }
}

export interface CatalogModel {
  id: string
  name: string
  description: string
  uncensored: boolean
  supportsTools: boolean
  vision: boolean
  reasoning: boolean
  codeOptimized: boolean
  webSearch: boolean
  contextTokens: number | null
  maxCompletionTokens: number | null
  privacy: 'private' | 'anonymized' | null
  traits: string[]
  beta: boolean
  /** Kullanımdan kalkma tarihi (ISO) */
  deprecatedAt: string | null
  price: ModelPrice | null
  aa: AaScores | null
  /** [g9] Sağlayıcı; yoksa kimliğin önekinden çıkarılır (ön eksiz = Venice) */
  provider?: ProviderId
}

export interface AaScores {
  id: string
  name: string
  /** false: ad tam eşleşmedi, boyut/tarih etiketleri yok sayılarak eşleşti */
  exact: boolean
  intelligence: number | null
  coding: number | null
  math: number | null
  tokensPerSecond: number | null
  ttftSeconds: number | null
}

export interface ProbeResult__model_probe {
  model: string
  text: string
  durationMs: number
  /** İlk metin parçasının geldiği an; metin hiç gelmediyse null */
  firstTokenMs: number | null
  promptTokens: number | null
  completionTokens: number | null
  costUsd: number | null
  finishReason: string | null
}

export interface G7Settings {
  /** Yalnızca kullanıcı dosyasındaki değerler (düzenlenebilir olanlar) */
  modelAliases: Record<string, string>
  taskModels: TaskModels
  theme: ThemeSetting
  /** [cdx] Bildirim tercihleri (masaüstü bildirimi, ses) */
  notifications: NotificationPrefs
  /** [cdx] Git tercihleri */
  git: GitPrefs
  /** [cdx] Editör tercihi */
  editor: EditorPrefs
  /** [cdx] Açık projenin betikleri (katmanlar birleşik) */
  scripts: ScriptPrefs
  /** [cdx] Kullanıcı tanımlı ajan profilleri (yerleşikler arayüzde eklenir) */
  profiles: AgentProfile[]
  /** [perf] Denetim günlüğü saklama (gün) */
  logs: { keepDays: number }
  app: {
    version: string
    tauri: string
    webview: string
    webviewName: string
    platform: string
    arch: string
  }
}

export interface TaskModels {
  explore?: string
  plan?: string
  review?: string
  title?: string
  compaction?: string
}

export type TaskKind = 'explore' | 'plan' | 'review' | 'title' | 'compaction'

export type ThemeSetting = 'system' | 'light' | 'dark'

export interface NotificationPrefs {
  desktop: boolean
  sound: boolean
  /** Uygulama içi bildirim sesi (Web Audio ile üretilir; dosya yok) */
  soundName: NotificationSound
}

export type NotificationSound = 'soft' | 'double' | 'bell' | 'chime'

export interface GitPrefs {
  /** Otomatik dal adı öneki (ör. "kor/" → kor/1a2b-gorev) */
  branchPrefix: string
  /** Worktree'lerin konulacağı klasör; boş = <depo>-worktrees kardeş klasörü */
  worktreeDir: string
}

export interface EditorPrefs {
  type: EditorType
  /** type=custom: çalıştırılacak komut; {file} ve {line} yer tutucuları */
  customCommand: string
  /** Boş değilse VS Code ailesi --remote ssh-remote+host ile açar */
  sshHost: string
}

export type EditorType = 'code' | 'cursor' | 'windsurf' | 'zed' | 'custom' | 'system'

export interface ScriptPrefs {
  setup: string
  cleanup: string
  dev: string
}

/** [cdx] Ajan yapılandırma profili: tek tıkla mod + model + akıl yürütme düzeyi (cdesktop "Default / Opus / Approvals / Plan"). */
export interface AgentProfile {
  name: string
  mode?: PermissionMode
  model?: string
  effort?: 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max'
}

export interface PermissionRulesInfo {
  allow: string[]
  deny: string[]
  /** Proje dosyalarından gelen deny kuralları (salt okunur) */
  projectDeny: string[]
  defaultMode: PermissionMode
  /** Kural yazılabilecek araç adları */
  tools: { name: string; kind: 'read' | 'edit' | 'exec' }[]
  userFile: string
}

export interface TrustInfo {
  root: string
  /** null: henüz karar verilmedi */
  decision: boolean | null
  projectFiles: string[]
  trusted: boolean
}

export interface PrivacyInfo {
  configDir: string
  sessionsDir: string
  sessionCount: number
  memoryFile: string
  memoryExists: boolean
  auditDir: string
  auditFiles: number
  auditBytes: number
  settingsFile: string
  trustFile: string
  aaCacheFile: string
  promptFile: string
  userDataDir: string
  /** safeStorage: işletim sisteminin anahtar deposuyla şifreli */
  keyEncryption: boolean
}

export type PrivacyFolder = 'config' | 'sessions' | 'audit' | 'userData'

export type PrivacyClearTarget = 'sessions' | 'audit'

export type UsagePeriod = 'day' | 'week'

export interface UsageHistory {
  period: UsagePeriod
  buckets: UsageBucket[]
  totalUsd: number
  totalDiem: number
  topModels: { name: string; usd: number; diem: number }[]
}

export interface UsageBucket {
  /** Kovanın ilk günü, YYYY-MM-DD */
  start: string
  usd: number
  diem: number
}

export interface RuntimeStatus {
  id: string
  name: string
  installed: boolean
  version: string | null
  winget: boolean
}

export interface InstallTick {
  phase: InstallPhase
  /** null: çıktıda yüzde yok, çubuk belirsiz kalır. 100 yalnız süreç başarıyla bitince. */
  percent: number | null
  line: string
}

export type InstallPhase = 'indiriliyor' | 'kuruluyor' | 'dogrulaniyor' | 'bitti' | 'hata'

export type AgentEvent = AgentEventBody & { sessionId: string }

export type G4Event = { type: 'fs'; root: string; paths: string[]; overflow: boolean } | { type: 'external'; sessionId: string; path: string; status: ExternalStatus | null }

export type ExternalStatus = 'modified' | 'deleted'

export type PreviewEvent = { type: 'dev'; status: DevServerStatus } | { type: 'nav'; url: string; title: string; canGoBack: boolean; loading: boolean } | { type: 'pick'; selection: PreviewSelection } | { type: 'console'; level: 'log' | 'warn' | 'error'; text: string } | { type: 'loadError'; url: string; message: string } | { type: 'open'; url: string | null }

/** Tarayıcı companion'ın seçtiği öğe: ajana "bu öğeyi değiştir" demek için yeterli bağlam. */
export interface PreviewSelection {
  id: string
  url: string
  tag: string
  selector: string
  text: string
  /** React/Vue geliştirme kaynağı (varsa) */
  source: { file: string; line: number; column: number | null } | null
  componentName: string | null
  rect: { width: number; height: number }
}

export type BrowserPanelEvent = { type: 'tabs'; tabs: BrowserTabInfo[] } | { type: 'open' } | { type: 'console'; level: 'log' | 'warn' | 'error'; text: string; tabId: string } | { type: 'action'; text: string }

export interface G14Alert {
  at: number
  sessionId: string
  kind: 'slowTool' | 'turnBudget' | 'memoryBudget'
  message: string
}

export interface AnalysisReport {
  /** security, compliance-kvkk, compliance-gdpr, licenses… */
  tool: string
  findings: Finding[]
  notes: string[]
}

/** Ortak analiz sözleşmesi (`kor_runtime::analysis::Finding`): her bulgu kural + kaynak + dosya:satır taşır. */
export interface Finding {
  rule: string
  /** Üreten tarayıcı: kor, gitleaks, semgrep, osv, spdx… */
  source: string
  severity: Severity
  /** Proje köküne göre */
  file: string
  /** 1 tabanlı */
  line: number
  column?: number
  message: string
  fix?: string
  cwe?: string
}

export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'info'

export type ExportFormat__analysis = 'md' | 'json' | 'sarif'

export interface ExportResult {
  /** İptalde null */
  path: string | null
  sha256?: string | null
  auditHash?: string | null
  /** `.minisig` yolu; anahtar yapılandırılmamışsa null */
  signature?: string | null
}

export interface VerifyResult {
  path: string | null
  ok?: boolean
  result?: { sha256: string; linked_at: string | null; chain_ok: boolean; signature_ok: boolean | null }
}

export interface HostItem {
  name: string
  kind?: string
  url?: string | null
  ssh?: string | null
  client?: string | null
  error?: string
}

export type WebArgs = Record<string, unknown>

export interface PlanRow {
  step: string
  target: string
  impact: string
  reversible: boolean
}

export interface RunReport {
  host: string
  plan: PlanRow[]
  approved: boolean
  applied: string | null
  output: string[]
  verify: VerifyResult__web | null
  rolled_back: boolean
  credential: string | null
  error: string | null
  secs: number
}

export interface VerifyResult__web {
  url: string
  status: number
  ms: number
  fatal: boolean
  new_log_errors?: number | null
  ok: boolean
  error?: string
}

export interface SiteItem {
  name: string
  url?: string
  rest?: boolean
  cli?: boolean
  host?: string | null
  client?: string | null
  error?: string
}

export interface ClientItem {
  id: string
  client: { name: string; email: string; sites: string[]; hosts: string[]; urls: string[] }
  openTickets: number
}

export type VisualTarget = 'react' | 'svelte' | 'tauri' | 'html'

/** `visual.screenshot` / `visual.diagram`: hiçbir şey yazılmadan dönen öneriler. */
export interface VisualResult {
  /** Görüntü modele gitti (vision); değilse `description` (yerel OCR + yerleşim) gitti */
  vision: boolean
  description: string | null
  /** Modelin tam yanıtı (bileşen ağacı başta) */
  reply: string
  provider: string
  files: ReviewFile[]
}

/** Aracın yazacağı bir dosya: after === null dosyayı siler. */
export interface ReviewFile {
  path: string
  before: string | null
  after: string | null
}

export type VisualizeFormat = 'html' | 'mermaid' | 'svg'

export interface VisualizeResult {
  format: VisualizeFormat
  main: string
  files: string[]
  counts: VisualizeCount[]
}

export interface VisualizeCount {
  view: 'modules' | 'calls' | 'dataflow'
  title: string
  nodes: number
  edges: number
  omitted: number
}

export interface VoiceStatus {
  ready: boolean
  whisper: string | null
  model: string | null
  /** İndirilecek doğrulanmış bileşenler; `size: null` bu makine için derleme yok */
  missing: { name: string; version: string; size: number | null }[]
}

/** `vault.list()`: adlar, kasanın yeri ve `[vault.scope]` (null = tüm komutlar, [] = hiçbiri). */
export interface VaultList {
  names: string[]
  backend: 'keychain' | 'file'
  file: string | null
  scope: Record<string, string[] | null>
}

export interface VaultImport {
  path: string
  /** İçe aktarılacak adlar */
  found: string[]
  /** Atlanan adlar ya da `#satır` */
  skipped: string[]
  /** Kasada zaten olan (üzerine yazılacak) adlar */
  existing: string[]
  imported: boolean
  names: string[]
}

export interface AuditQuery {
  action?: string
  model?: string
  file?: string
  offset?: number
  limit?: number
}

export interface AuditPage {
  path: string
  total: number
  entries: AuditEntry[]
  actions: string[]
  models: string[]
}

/**
 * `kor_agent::audit` satırı. `tool`: araç çağrısı; `task`: kullanıcı mesajı; `model`: model çağrısı;
 * `sandbox_denied`: sandbox engellemesi.
 */
export interface AuditEntry {
  ts: string
  action: string
  model?: string
  root?: string
  prev: string
  hash: string
  tool?: string
  /** `read` | `write` | `command` | `network` | `tool` */
  kind?: string
  target?: string
  decision?: 'allow' | 'deny' | 'approved' | 'rejected'
  ok?: boolean
  text?: string
  outcome?: string
  input?: number
  output?: number
  provider?: string
  /** Yaklaşık istek boyutu (bayt) */
  bytes?: number
  /** İstekte maskelenen sır sayısı */
  masked?: number
  /** Ondalık dize (`0.001200`); `cost_estimated` ise katalog fiyatından tahmin (≈) */
  cost_usd?: string
  cost_estimated?: boolean
}

export interface AuditVerify {
  path: string
  entries: number
  /** Gezilen dosya sayısı (arşivler + etkin dosya) */
  files: number
  ok: boolean
  /** İlk bozuk satır (`file` içinde, 1'den başlar) */
  line: number | null
  reason: 'json' | 'prev' | 'hash' | null
  file: string | null
  /** `audit.head` karşılaştırması; `truncated` sondan silme demektir */
  head: AuditHead
  /** En eski arşivler silinmiş (saklama) */
  trimmed: boolean
}

export type AuditHead = 'ok' | 'behind' | 'missing' | 'truncated'

/** `kor_agent::audit::Report` (yalnızca sayılar ve yollar; içerik yok). */
export interface AuditReport {
  since: string | null
  first: string | null
  last: string | null
  entries: number
  tasks: number
  providers: {
    provider: string
    calls: number
    bytes: number
    input: number
    output: number
    local: boolean | null
    jurisdiction: string | null
    training_on_data: boolean | null
    retention: string | null
  }[]
  masked: number
  sandbox_denied: number
  reads: number
  writes: number
  commands: number
}

/** ADR 0033: `mode = "on"` iken bu makinede neyin gerçekten uygulandığı. */
export interface SandboxStatus__security {
  mode: SandboxMode
  preference: SandboxPreference
  /** `bubblewrap`, `seatbelt`, `appcontainer`, `docker`, `none` */
  backend: string
  enforcing: boolean
  /** Proje ve özel geçici klasör dışına yazma engelleniyor */
  files: boolean
  network: {
    on: boolean
    /** `host:port`; boşsa ağ açıkken her yer */
    allow: string[]
    /** `domains`: sunucu adları uygulanır (komut yalnızca süzen vekile ulaşır); `proxy`: vekil değişkenlerini kullanan programlar süzülür; `none`: ağ ya hep ya hiç */
    filter: 'none' | 'domains' | 'proxy'
  }
  limits: { memoryMb: number; memoryEnforced: boolean; cpuPercent: number; cpuEnforced: boolean }
  /** Yazmalar atılan bir katmana gider; proje değişmez */
  overlay: boolean
  image: string
  docker: { installed: boolean; daemon: boolean; imagePresent: boolean } | null
  /** Yerelleştirilmiş uyarılar (geçersiz girdi, süzülemeyen izin listesi, eksik imaj) */
  notes: string[]
}

export type SandboxMode = 'on' | 'off' | 'ask'

/** `[sandbox] backend`: `auto` yerel arka uç, yoksa kuruluysa Docker. */
export type SandboxPreference = 'auto' | 'native' | 'docker'

export interface SandboxProbe {
  backend: string
  /** Proje dışındaki dosya oluşmadı */
  blocked: boolean
  target: string
  /** Çıktıda tanınan engel satırları; boşsa engel denetim kaydına geçmezdi */
  detected: string[]
  output: string
}

export interface TeamStatus {
  /** `[team] server`, yoksa `KOR_TEAM_MEMORY_URL` */
  server: string | null
  source: 'config' | 'env' | null
  /** Anahtarlıkta erişim belirteci var mı (değer gelmez) */
  token: boolean
  privacy: boolean
}

export interface TeamCheck {
  reachable: boolean
  status?: number
  error?: string
}

export interface ShellChange {
  shell: string
  path: string
  /** Başlangıç dosyasına eklenecek / çıkarılacak blok */
  block: string
  /** false: dosyada değişecek bir şey yok */
  changed: boolean
  written: boolean
}

export interface ShellStatus {
  shells: { shell: string; path: string; installed: boolean; present: boolean; isDefault: boolean }[]
  record: string | null
  last: ShellFailure | null
}

/** `kor_agent::shellint::Failure`: son başarısız komut (kabuk kancası ya da `shell.capture`). */
export interface ShellFailure {
  command: string
  exit: number
  shell: string
  cwd: string
  /** Son çıktı satırları */
  output: string[]
}

export interface CommitProposal {
  files: string[]
  scope: string | null
  message: string
  header: string
  breaking: boolean
  parts: { files: string[]; message: string }[]
}

/** `triggers.list()` satırı (`triggers::describe`). */
export interface TriggerRow {
  name: string
  event: TriggerEvent
  /** Dal, zamanlama, yol ya da `kaynak ~ desen` */
  on: string
  task: string
  root: string
  /** Tanımlandığı ayar dosyası */
  origin: string
  enabled: boolean
  model: string
  budgetUsd: number
  approval: TriggerApproval
  pr: boolean
  secret: string
  branch: string
  schedule: string
  path: string
  source: string
  pattern: string
  debounceSecs: number
  /** Çalışamıyorsa nedeni */
  error: string | null
}

export type TriggerEvent = 'git_push' | 'cron' | 'webhook' | 'log_match'

export type TriggerApproval = 'notify' | 'auto' | 'deny'

/** `triggers.save` girdisi. */
export interface TriggerInput {
  name: string
  event: TriggerEvent
  task: string
  enabled: boolean
  branch?: string
  schedule?: string
  path?: string
  secret?: string
  source?: string
  pattern?: string
  debounceSecs?: number
  model?: string
  budgetUsd?: number
  approval: TriggerApproval
  pr: boolean
}

export interface TriggerJob {
  id: string
  trigger: string
  event: string
  detail: string
  root: string
  status: JobStatus
  created: number
  started: number | null
  finished: number | null
  base: string | null
  sha: string | null
  worktree: string | null
  transcript: string | null
  branch: string | null
  pr_url: string | null
  cost_usd: number
  cost_estimated: boolean
  approvals: number
  summary: string
  error: string | null
}

export type JobStatus = 'queued' | 'running' | 'waiting' | 'done' | 'failed'

export interface DaemonStatus {
  running: boolean
  pid?: number
  addr?: string
  started?: number
  version?: string
  queued?: number | null
  triggers?: number | null
  waiting?: WaitingApproval[] | null
}

export interface WaitingApproval {
  id: string
  job: string
  summary: string
}

export interface NotifySettings {
  telegram_chat: string
  slack: boolean
  email_to: string
  email_from: string
  smtp_host: string
  smtp_port: number
  smtp_user: string
  approval_timeout_mins: number
  publicUrl: string
  listen: string
  /** Kasada var mı (değerler arayüze gelmez) */
  secrets: { KOR_TELEGRAM_TOKEN: boolean; KOR_SLACK_WEBHOOK: boolean; KOR_SMTP_PASSWORD: boolean }
}

export interface NotifyTest {
  sent: string[]
  errors: string[]
  notes: string[]
}

export interface ToolEntry {
  name: string
  path: string
  version?: string | null
  description: string
  examples: string[]
  network: boolean
  known: boolean
}

export interface TypedTool {
  name: string
  origin: 'user' | 'project' | 'team' | 'generated'
  safety: 'read' | 'write' | 'network' | 'destructive'
  command: string[]
  description: string
}

export interface ToolHelp {
  name: string
  path: string
  source: string
  usage: string
  summary: string
  subcommands: [string, string][]
  flags: [string, string][]
  model_summary?: string | null
  excerpt: string
}

export interface TaskState {
  version: number
  id: string
  goal: string
  summary: string
  status: TaskStatus
  created_ms: number
  updated_ms: number
  base: string
  branch: string
  settings: TaskSettings
  subtasks: SubTask[]
  conflicts: TaskConflict[]
  test: TaskTestRun | null
  orchestrator_usd: number
  reviewer_usd: number
  estimated: boolean
  forked_from: string | null
  error: string | null
}

export type TaskStatus = 'planning' | 'running' | 'merging' | 'conflict' | 'done' | 'failed' | 'cancelled'

export interface TaskSettings {
  agents: number
  max_parallel: number
  models: Record<string, string>
  budget_usd: number
  budget_per_agent_usd: number
  test_command: string
  max_turns: number
}

export interface SubTask {
  index: number
  role: string
  goal: string
  files: string[]
  status: SubStatus
  branch: string
  worktree: string
  model: string | null
  models_used: string[]
  base: string | null
  commit: string | null
  changed: string[]
  outside: string[]
  cost_usd: number
  estimated: boolean
  test: TaskTestRun | null
  review: string | null
  error: string | null
  session: string | null
}

export type SubStatus = 'pending' | 'running' | 'approved' | 'rejected' | 'failed' | 'merged' | 'conflict' | 'dropped' | 'cancelled'

export interface TaskTestRun {
  ok: boolean
  output: string
}

export interface TaskConflict {
  index: number
  role: string
  files: ConflictFile[]
  diff: string
}

export interface ConflictFile {
  path: string
  ours: string
  theirs: string
}

export interface TaskStartOptions {
  goal: string
  agents?: number
  parallel?: number
  budgetUsd?: number
  test?: string
  /** rol → `provider/model` */
  models?: Record<string, string>
}

export interface ShellState {
  overview: Overview__shellui
  unread: number
  /** Bulunan yeni sürüm */
  update: string | null
  /** Son 3 proje */
  recent: string[]
  pip: { opacity: number; visible: boolean }
  /** Görev çubuğuyla aynı: null boşta, 'indeterminate' ilerleme bilinmiyor, sayı yüzde */
  progress: number | 'indeterminate' | null
}

export interface Overview__shellui {
  desktop: boolean
  sessions: RunningSession[]
  daemon: { running: boolean; queued: number; jobs: DaemonJob[] }
  remote: RemoteTask[]
  approvals: QueueItem[]
  groups: QueueGroup[]
  paused: boolean
  now: number
}

export interface RunningSession {
  id: string
  title: string
  root: string
  since: number
  /** Çekirdeğin `agent.status` fiili */
  verb: string
  tool: string | null
  costUsd: number
  costEstimated: boolean
  /** [bitti, toplam] yapılacaklar listesinden */
  progress: [number, number] | null
  eta: Eta | null
  waiting: boolean
}

export interface Eta {
  /** Unix ms; 90. yüzdelik aşıldıysa null */
  finishMs: number | null
  remainingSecs: number | null
  percentile: number
  scope: EtaScope
  samples: number
  overdue: boolean
}

export type EtaScope = 'project' | 'all'

export interface DaemonJob {
  id: string
  trigger: string
  status: string
  root: string
  started: number | null
  costUsd: number
  costEstimated: boolean
  eta: Eta | null
}

export interface RemoteTask {
  id: string
  state: string
  project: string
  prompt: string
  started: number
}

export interface QueueItem {
  id: string
  source: ApprovalSource
  /** Sohbet, daemon işi ya da uzak görev kimliği */
  owner: string
  title: string
  tool: string
  summary: string
  destructive: string | null
  files: string[]
  since: number
  risk: Risk
  group: string
}

export type ApprovalSource = 'desktop' | 'daemon' | 'remote'

export type Risk = 'low' | 'medium' | 'high'

export interface QueueGroup {
  key: string
  tool: string
  summary: string
  ids: string[]
  risk: Risk
}

export interface KorNotification {
  id: string
  /** Unix ms */
  at: number
  category: NotificationCategory
  title: string
  body: string
  session: string | null
  job: string | null
  approval: string | null
  costUsd: number | null
  /** Katalog fiyatından tahmin (≈) */
  costEstimated: boolean
  files: number | null
  read: boolean
  key: string | null
}

export type NotificationCategory = 'approval' | 'question' | 'done' | 'error' | 'budget' | 'update' | 'health' | 'component' | 'quota'

export interface ShellSettings {
  notify: {
    toast: boolean
    bell: boolean
    flash: boolean
    sound: boolean
    idleSecs: number
    quietHours: string
    muted: NotificationCategory[]
  }
  tray: { enabled: boolean; closeToTray: boolean }
  pip: { autoShow: boolean; hideFullscreen: boolean; opacity: number }
}

export type ShellSettingKey = 'notify.toast' | 'notify.bell' | 'notify.flash' | 'notify.sound' | 'notify.idle_secs' | 'notify.quiet_hours' | 'notify.muted' | 'tray.enabled' | 'tray.close_to_tray' | 'pip.auto_show' | 'pip.hide_fullscreen' | 'pip.opacity'

export interface Alert {
  inline: boolean
  toast: boolean
  sound: boolean
  flash: boolean
  bell: boolean
}

export type SeoDepth = 'quick' | 'full' | 'forensic'

export interface SeoAuditResult {
  report: SeoReport
  rich: SeoRich[]
  diff: SeoDiff | null
  pages: number
}

export interface SeoReport {
  tool: string
  findings: SeoFinding[]
  notes: string[]
}

export interface SeoFinding {
  rule: string
  source: string
  severity: SeoSeverity
  /** URL (ya da log dosyası / `competitor:<ad>`) */
  file: string
  line: number
  column?: number
  message: string
  fix?: string
}

export type SeoSeverity = 'critical' | 'high' | 'medium' | 'low' | 'info'

export interface SeoRich {
  url: string
  eligible: string[]
  /** [tür, eksik zorunlu alanlar] */
  blocked: [string, string[]][]
}

export interface SeoDiff {
  new: SeoFinding[]
  fixed: SeoFinding[]
}

export interface SeoGscStatus {
  client: boolean
  signed_in: boolean
}

export interface SeoRow {
  keys: string[]
  clicks: number
  impressions: number
  ctr: number
  position: number
}

export interface SeoDrop {
  page: string
  clicks_before: number
  clicks_now: number
  impressions_before: number
  impressions_now: number
  position_before: number
  position_now: number
  ctr_before: number
  ctr_now: number
  /** `position` | `demand` | `snippet` | `gone` ya da denetim kuralları */
  hints: string[]
}

export interface SeoBingStat {
  key: string
  clicks: number
  impressions: number
  avg_click_position: number
  avg_impression_position: number
}

export interface SeoVerified {
  fixed: SeoFinding[]
  remaining: SeoFinding[]
  new: SeoFinding[]
  urls: number
}

export interface SeoContentState {
  slug: string
  keyword: string
  lang: string
  stage: SeoContentStage
  created: string
  approved_sha: string | null
  published: unknown
  model: string | null
}

export type SeoContentStage = 'new' | 'brief' | 'outline' | 'draft' | 'checked' | 'approved' | 'published'

export interface SeoContentFiles {
  state: SeoContentState
  brief: string
  outline: string
  draft: string
  check: string
  dir: string
}

export interface SeoMetaRow {
  kind: 'page' | 'image'
  url: string
  page: string
  current_title: string
  current_description: string
  current_alt: string
  title: string
  description: string
  alt: string
  context: string
  issues: string
}

export interface SeoMetaEstimate {
  model: string
  inputTokens: number
  outputTokens: number
  /** null: katalogda fiyat yok */
  usd: number | null
  estimated: true
}

export interface SeoApplied {
  applied: { changed: string[]; skipped: [string, string][] }
  undo: string
}

export interface SeoLogsResult {
  files: { file: string; stats: SeoLogStats }[]
  report: SeoReport
}

export interface SeoLogStats {
  lines: number
  parsed: number
  first: string | null
  last: string | null
  status: Record<string, number>
  bots: Record<string, SeoBotStats>
  googlebot_per_day: Record<string, number>
  waste: Record<string, number>
  p50_ms: number | null
  p95_ms: number | null
  never_crawled: string[]
  not_in_sitemap: string[]
  dropping: [string, number, number][]
  fake_ips: string[]
  dns_checked: boolean
}

export interface SeoBotStats {
  hits: number
  verified: number
  fake: number
  unverified: number
  status: Record<string, number>
}

export interface SeoCompetitor {
  name: string
  urls: string[]
  sitemap: boolean
  ours: string
}

export interface SeoCompDiff {
  name: string
  new_urls: string[]
  removed_urls: string[]
  changes: { url: string; field: string; before: string; after: string }[]
  gaps: string[]
}

export interface SeoTeamReport {
  report: SeoReport
  agents: { id: string; label: string; findings: number; notes: string[] }[]
  priorities: SeoPriority[]
  conflicts: [string, string][]
  fixes_markdown: string
  htaccess: string
  gsc_urls: string[]
  diff: SeoDiff | null
  summary: string | null
}

export interface SeoPriority {
  rule: string
  agent: string
  severity: SeoSeverity
  pages: number
  effort: number
  score: number
  fix: string
}

export interface SeoOpportunity {
  keys: string[]
  impressions: number
  clicks: number
  ctr: number
  position: number
  target: number
  expected_ctr: number
  ctr_source: 'site' | 'default'
  band: number
  extra_clicks: number
  score: number
  index: number
}

export interface SeoProgress {
  kind: string
  detail?: string
  done?: number
  total?: number
}

export type SeoAgentEvent = { state: 'started'; agent: string } | { state: 'progress'; agent: string; detail: string } | { state: 'done'; agent: string; findings: number } | { state: 'skipped'; agent: string; reason: string }

export interface ReplaySummary {
  id: string
  /** Cast başlığındaki Unix saniyesi */
  started: number | null
  /** İlk ve son olay arası saniye */
  duration: number
  prompts: number
  /** İlk istemin ilk satırı */
  title: string
  tools: number
  events: number
  costUsd: number
  /** Bir çağrı katalogdan fiyatlandıysa (≈) */
  costEstimated: boolean
  bytes: number
}

export interface ReplayRecording {
  id: string
  header: unknown
  steps: ReplayStep[]
  events: ReplayEvent[]
}

export interface ReplayStep {
  /** `events` içindeki sıra */
  index: number
  t: number
  kind: string
  title: string
}

/** events.jsonl satırı; alanlar `kind`'a göre değişir (prompt, text, diff, ask, approval, tool_start, ...) */
export interface ReplayEvent {
  kind: string
  t: number
  [key: string]: unknown
}

export interface RemoteHost {
  /** connections.toml kimliği ya da adres */
  id: string
  address: string
  host: string
  /** Son keşif (Unix saniye) */
  scannedAt: number | null
}

export interface RemoteStatus {
  target: string
  task: string | null
}

export interface RemoteConnected {
  target: string
  os: string
  arch: string
  version: string | null
  /** Değiştirilen sürüm ('' = ilk kurulum); null = zaten aynı sürüm */
  replaced: string | null
  project: string
  tasks: RemoteTask__remote[]
}

export interface RemoteTask__remote {
  id: string
  state: 'queued' | 'running' | 'waiting' | 'idle' | 'failed' | string
  project: string
  prompt: string
  started: number
  updated: number
  events: number
}

export interface Discovery {
  snapshot: InfraSnapshot
  sudoOffered: SudoOffer[]
  sudoUsed: string[]
  markdown: string
  linked: boolean
}

export interface InfraSnapshot {
  host: string
  address: string
  taken_at: number
  hostname: string
  os: string
  kernel: string
  arch: string
  services: InfraService[]
  ports: { port: number; address: string; process: string }[]
  vhosts: { server: string; names: string[]; upstreams: string[]; services: string[] }[]
  schedules: { kind: string; when: string; what: string }[]
  disks: { mount: string; size: string; used: string; avail: string; use_pct: string }[]
  memory: {
    total_mb: number
    used_mb: number
    available_mb: number
    swap_total_mb: number
    swap_used_mb: number
  } | null
  languages: { name: string; version: string }[]
  env_files: string[]
  notes: string[]
}

export interface InfraService {
  kind: string
  name: string
  state: string
  detail: string
  image: string | null
  compose_project: string | null
  compose_service: string | null
  ports: number[]
  domains: string[]
}

export interface SudoOffer {
  step: string
  command: string
  reason: string
}

export interface RemoteEvent {
  target: string
  event: RemoteWireEvent
}

/** Masaüstü sohbetiyle aynı olay şeması + uzak kipe özgü `attached`, `confirmed`, `tasks`, `detached`. */
export interface RemoteWireEvent {
  type: string
  task?: string
  seq?: number
  [key: string]: unknown
}

export interface AutoRow {
  /** `compact` → yapılandırma anahtarı `auto.compact` */
  key: string
  value: boolean
  source: string
  fromDefault: boolean
  /** Davranışın çalışması için açık olması gereken eski anahtarlar ve değerleri */
  related: [string, string][]
}

export interface UiPrefs {
  wrap: boolean
  wrapWidth: number
  maxWidth: LineWidth
  density: Density
  fontSize: number
  lineHeight: number
  codeFont: string
  diffView: DiffViewMode
  toolOutput: ToolOutput
  screenReader: boolean
  highContrast: boolean
  reducedMotion: boolean
  keys: KeyMode
}

export type LineWidth = 'narrow' | 'medium' | 'wide' | 'full'

export type Density = 'comfortable' | 'compact'

export type DiffViewMode = 'inline' | 'split'

export type ToolOutput = 'quiet' | 'normal' | 'verbose'

export type KeyMode = 'default' | 'vim' | 'emacs'

export interface AgentPrefs {
  askThreshold: AskThreshold
  compactConfirm: boolean
  planLock: boolean
}

export type AskThreshold = 'never' | 'critical' | 'often'

export interface Keymap {
  mode: 'Default' | 'Vim' | 'Emacs' | null
  tui: Record<string, string>
  composer: Record<string, string>
  files: string[]
  warnings: string[]
}

export interface QuestionAnswer {
  choice: number
  remember: boolean
}

export type CompactAnswer = { keep: number[] } | { cancel: true }

export interface DoctorFix {
  fixed: string[]
  failed: string[]
  output: string
}

export interface PlanInfo {
  id: string
  title: string
  status: 'draft' | 'approved'
  session: string | null
  created: number
  body: string
  path: string
  files: string[]
}

export interface SessionFilter {
  tag?: string
  folder?: string
  pinned?: boolean
  archived?: boolean
}

export interface SessionEntry {
  id: string
  title: string
  folder: string | null
  tags: string[]
  pinned: boolean
  archived: boolean
  color: string | null
  root: string | null
  modified: number
}

export interface SessionHit {
  id: string
  title: string
  snippet: string
  score: number
  semantic: boolean
}

export type SessionColor = 'red' | 'orange' | 'yellow' | 'green' | 'blue' | 'purple' | 'gray'

export interface PerfRunOptions {
  target: string
  profiler?: string
  duration?: number
  requests?: number
  concurrency?: number
  pid?: number
  cmd?: string
  allowHosts?: string[]
  baseline?: string
}

export interface PerfReport {
  id: string
  created: string
  target: string
  kind: 'command' | 'test' | 'endpoint' | 'file'
  language: string | null
  profiler: string
  command: string | null
  /** Profillenen komutun korumalı alanı (`none` = dışında) */
  sandbox: string
  exitCode: number | null
  wallMs: number | null
  unit: PerfUnit
  total: number
  top: PerfRow[]
  hottest: PerfRow | null
  memory: { unit: PerfUnit; total: number; count: number | null; top: PerfRow[] } | null
  memoryNote: string | null
  load: PerfLoadStats | null
  flamegraph: string
  memoryFlamegraph: string | null
  collapsed: string
  notes: string[]
  baseline: string | null
  compare: PerfCompareRow[]
}

export type PerfUnit = 'samples' | 'nanoseconds' | 'microseconds' | 'bytes'

export interface PerfRow {
  name: string
  /** Proje içindeyse köke göre, değilse profiler'ın yazdığı gibi */
  file: string | null
  line: number | null
  selfWeight: number
  totalWeight: number
  selfPct: number
  totalPct: number
  node: PerfNodeLink | null
}

export interface PerfNodeLink {
  id: string
  name: string
  /** Proje köküne göre yol */
  file: string
  line: number | null
  /** `line`: dosya:satır düğümün aralığında; `name`: aynı ad */
  by: 'line' | 'name'
}

export interface PerfLoadStats {
  requests: number
  ok: number
  errors: number
  wallMs: number
  rps: number
  p50Ms: number
  p95Ms: number
  maxMs: number
  firstError: string | null
}

export interface PerfCompareRow {
  metric: 'wall_ms' | 'samples' | 'top_share' | 'rps' | 'p50_ms' | 'p95_ms' | 'memory'
  subject: string | null
  before: number
  /** null: sonraki ölçümde bulunamadı (ör. fonksiyon ilk tablodan çıktı) */
  after: number | null
  changePct: number | null
}

export interface PerfSummary {
  id: string
  created: string
  target: string
  kind: 'command' | 'test' | 'endpoint' | 'file'
  profiler: string
  wallMs: number | null
  hottest: string | null
  hottestPct: number | null
}

export interface PackagesList {
  root: string
  kinds: PackageTool[]
  packages: PackageView[]
}

export type PackageTool = 'cargo' | 'pnpm' | 'yarn' | 'npm' | 'lerna' | 'nx' | 'turbo' | 'bazel' | 'buck' | 'go' | 'gradle' | 'dotnet'

export interface PackageView {
  name: string
  tool: PackageTool
  /** Çalışma alanı köküne göre, `/` ayraçlı */
  root: string
  manifest: string
  deps: string[]
}

export interface PackagesAffected {
  changed: string[]
  packages: PackageView[]
  /** Daraltılmış derleme/test komutları (Kor çalıştırmaz, yalnızca yazar) */
  commands: string[]
}

export interface PackageDeps {
  package: PackageView
  deps: string[]
  dependents: string[]
}

/** `model` alanı paket adıdır; `"-"` = hiçbir pakette değil */
export interface PackageCostRow {
  model: string
  requests: number
  prompt: number
  completion: number
  usd: number
  estimated: boolean
  unpriced: number
}

export interface CostRow {
  model: string
  requests: number
  prompt: number
  completion: number
  usd: number
  /** Tutarın bir kısmı katalog fiyatından (≈) */
  estimated: boolean
  /** Fiyatı bilinmeyen istek sayısı (usd'ye girmez) */
  unpriced: number
}

export type OsintRequest = { kind: 'open'; root: string; files: string[]; origin: OsintOrigin } | {
  kind: 'task'
  prompt: string
  root: string | null
  files: string[]
  quick: QuickTask | null
  origin: OsintOrigin
} | { kind: 'resume'; id: string; origin: OsintOrigin } | { kind: 'edit'; files: string[]; origin: OsintOrigin } | { kind: 'terminal'; root: string; files: string[] } | { kind: 'palette' } | { kind: 'new' } | { kind: 'running' }

/** shell: kullanıcının kendi tıklaması; link: `kor://` bağlantısı (her sayfa açabilir, onay istenir). */
export type OsintOrigin = 'shell' | 'link'

export type QuickTask = 'explain' | 'bugs' | 'test'

export interface HotkeyStatus {
  /** Ayardaki değer; boş = kapalı */
  hotkey: string
  default: string
  /** Şu an işletim sistemine kayıtlı */
  active: boolean
}

/** `kor_platform::shell_integration::Status` (`kor integrate status` ile aynı). */
export interface MenuStatus {
  installed: boolean
  extended?: boolean
  [key: string]: unknown
}

export interface OsExtraList {
  os: OsName
  items: OsExtraStatus[]
  /** Windows: kurulu WSL dağıtımları. */
  wsl: string[]
}

export type OsName = 'windows' | 'macos' | 'linux' | 'other'

export interface OsExtraStatus {
  id: string
  os: OsName
  kind: OsExtraKind
  /** `delegated` için kuran yüzey (`desktop`, `kor integrate`). */
  via: string | null
  installed: boolean
  available: boolean
  /** Neden kodu (`no-powertoys`, `shortcuts-manual`…). */
  reason: string | null
}

/** `install`: Kor kurar/kaldırır; `desktop`: masaüstü çalışırken uygular; `builtin`: hep açık; `delegated`: başka yüzeyden. */
export type OsExtraKind = 'install' | 'desktop' | 'builtin' | 'delegated'

export interface PortalOutcome {
  portal: 'screencast' | 'remote-desktop' | 'file-chooser' | 'notification'
  granted: boolean
  detail: string | null
}

export interface OfflineStatus {
  enabled: boolean
  runtime: string
  endpoint: string
  model: string
  servers: OfflineServer[]
  runtimeReady: boolean
  llamaServer: string | null
  models: OfflineModel[]
  embedding: boolean
  catalogAgeSecs: number | null
  ready: boolean
  missing: OfflineMissing[]
  ollamaModels: string[]
  /** Kor'un başlattığı llama-server'ın modeli */
  serving: string | null
}

export interface OfflineServer {
  provider: string
  name: string
  base_url: string
  models: string[]
}

export interface OfflineModel {
  /** Bileşen adı (`model-qwen3-4b`) */
  name: string
  about: string
  size: number | null
  installed: boolean
  /** `local-llamacpp/qwen3-4b` */
  model_ref: string
}

export type OfflineMissing = 'model' | 'runtime' | 'model-file'

export interface OfflineInitOptions {
  /** `llamacpp`, `ollama`, `lmstudio` ya da sağlayıcı kimliği */
  runtime?: string
  models?: string[]
  defaultModel?: string
  embedding?: boolean
  catalog?: boolean
}

export interface OfflinePlan {
  runtime: string
  endpoint: string
  models: string[]
  defaultModel: string
  downloads: OfflineDownload[]
  embedding: boolean
  catalog: boolean
}

export interface OfflineDownload {
  step: 'runtime' | 'model' | 'embedding'
  name: string
  size: number | null
}

export interface OfflineBundleOptions {
  models?: string[]
  /** `win-x64`, `linux-arm64`, … (boşsa bu makine) */
  platform?: string
  embedding?: boolean
  /** Önce bir masaüstü kurulum dosyası seçtirir */
  desktop?: boolean
}

export interface OfflineEvent {
  step: OfflineStep
  name: string
  state: 'start' | 'progress' | 'done' | 'skip' | 'error'
  done: number
  total: number | null
  message: string | null
}

export type OfflineStep = 'runtime' | 'model' | 'embedding' | 'catalog' | 'config' | 'bundle' | 'import'

export interface OfflineDone {
  job: 'init' | 'bundle' | 'import'
  ok: boolean
  result?: unknown
  error?: string
}

export type ExplainLevel = 'junior' | 'normal' | 'expert'

export interface Archaeology {
  file: string
  lang: string | null
  stats: { lines: number; functions: { name: string; line: number; end: number; complexity: number }[] } | null
  tracked: boolean
  commits: number
  first: CommitRef | null
  last: CommitRef | null
  age_days: number | null
  last_change_days: number | null
  churn_90d: number
  churn_365d: number
  authors: Author[]
  renames: { commit: CommitRef; from: string; to: string }[]
  refactors: CommitRef[]
  danger: DangerZone[]
  graph: {
    community: string | null
    dependents: string[]
    co_changes: [string, number][]
    dependents_total: number
  } | null
  sources: string[]
}

export interface CommitRef {
  hash: string
  time: number
  subject: string
  author: string
}

export interface Author {
  name: string
  email: string
  commits: number
  lines: number
  share: number
}

export interface DangerZone {
  name: string
  line: number
  end: number
  complexity: number
  commits: number
  recent_lines: number
  score: number
}

export interface QualityReport {
  commit: string | null
  dirty: boolean
  taken_at: number
  overall: number | null
  grade: Grade | null
  dimensions: Dimension[]
  debt_hours: number
  files: number
  findings: Finding[]
  findings_total: number
  trend: QualityTrend | null
}

export type Grade = 'A' | 'B' | 'C' | 'D' | 'F'

export interface Dimension {
  key: DimensionKey
  score: number | null
  grade: Grade | null
  formula: string
  metrics: Metric[]
  /** `no-coverage`, `coverage-stale`, `no-perf`, `perf-unreadable`, `perf-no-baseline`, `no-source` */
  note: string | null
}

export type DimensionKey = 'maintainability' | 'coverage' | 'documentation' | 'security' | 'performance'

export interface Metric {
  key: string
  value: number
  /** `%`, `count`, `bool`, `cc`, `ms` */
  unit: string
  source: string
}

export interface QualityTrend {
  since: string
  since_at: number
  overall: number | null
  dimensions: [string, number][]
  debt_hours: number
}

export interface QualitySnapshot {
  commit: string
  dirty: boolean
  taken_at: number
  overall: number | null
  dimensions: [string, number | null][]
  debt_hours: number
  coverage: number | null
}

export interface Debt {
  formula: string
  total_hours: number
  items: DebtItem[]
  growth: { per_month: number; basis: GrowthBasis } | null
  threshold_hours: number
  months_to_threshold: number | null
  files_scanned: number
}

export interface DebtItem {
  file: string
  hours: number
  complexity_excess: number
  worst: [string, number, number] | null
  code_lines: number
  duplicated_lines: number
  churn: number
  test_factor: number
  test: TestBasis
}

export type TestBasis = { kind: 'coverage'; percent: number; source: string } | { kind: 'not_in_report'; source: string } | { kind: 'tests_found' } | { kind: 'no_tests' }

export type GrowthBasis = { kind: 'snapshots'; count: number; from: string; to: string } | { kind: 'git'; rev: string }

export interface StyleProfile {
  author: string
  commits: number
  files: number
  lines: number
  functions: number
  languages: Record<string, number>
  naming: Convention[]
  errors: Record<string, number>
  early_return_share: number
  avg_function_lines: number
  avg_complexity: number
  comment_density: number
  doc_share: number
  constructs: Record<string, number>
  preferences: string[]
  source: string
  taken_at: number
}

export interface Convention {
  lang: string
  category: string
  dominant: string
  share: number
  samples: number
}

export interface DevStats {
  author: string
  weeks: StatsWeek[]
  streak: number
  longest_streak: number
  sources: string[]
  team: TeamRow[] | null
}

export interface StatsWeek {
  start: number
  end: number
  commits: number
  human: StatsLines
  assisted: StatsLines
  agent: StatsLines
  agent_commits: number
  agent_writes: number
  fixes: number
  features: number
  types: Record<string, number>
  active_days: number
  coverage: [number, number] | null
}

export interface StatsLines {
  added: number
  deleted: number
}

export interface TeamRow {
  author: string
  commits: number
  added: number
  deleted: number
  fixes: number
  features: number
}

export type ImpactMode = 'name' | 'working' | 'staged'

export interface ImpactReport {
  target: string
  changed: string[]
  not_found: string[]
  files: string[]
  symbols: number
  endpoints: Endpoint[]
  migrations: string[]
  tables: string[]
  packages: string[]
  tests: string[]
  hours: number
  formula: string
  depth: number
  truncated: boolean
}

export interface Endpoint {
  rule: string
  file: string
  line: number
  name: string
}

export interface DeadReport {
  symbols: number
  entries: number
  entry_rules: Record<string, number>
  dead: DeadItem[]
  ambiguous: DeadItem[]
  notes: string[]
}

export interface DeadItem {
  id: string
  kind: string
  name: string
  file: string
  line: number
  end: number
  confidence: DeadConfidence
  reason: DeadReason
  unchanged_days: number | null
}

export type DeadConfidence = 'high' | 'medium' | 'ambiguous'

export type DeadReason = { kind: 'unreachable' } | { kind: 'name_elsewhere'; count: number } | { kind: 'reflection' }

export interface PrunePlan {
  branch: string
  base: string
  removed: DeadItem[]
  restored: DeadItem[]
  skipped: DeadItem[]
  check: CmdRun | null
  tests: CmdRun | null
  commit: string | null
  ready: boolean
  /** `prune-…` kimlikleri ya da ham hata metni */
  notes: string[]
}

export interface CmdRun {
  command: string
  ok: boolean
  output: string
}

export interface RecallHit {
  record: KnowledgeRecord
  score: number
}

export interface KnowledgeRecord {
  id: number
  /** Unix saniye */
  ts: number
  project: string
  kind: string
  task: string
  summary: string
  files: string[]
  decisions: string[]
  errors: string[]
  commands: string[]
  rules: string[]
}

export interface Decision {
  title: string
  body: string
  /** YYYY-MM-DD ya da boş */
  date: string
  refs: string[]
}

export interface MemoryOverview__memory {
  project: string
  dir: string
  docs: { file: MemoryDocFile; exists: boolean }[]
  /** YYYY-MM-DD ya da `undated`, yeniden eskiye */
  sessions: string[]
  gitNotes: boolean
}

export type MemoryDocFile = 'architecture.md' | 'decisions.md' | 'patterns.md' | 'hotspots.md'

/** Kor'un belgeye yazdığı işaretli bölüm; `edited`: kullanıcı değiştirdi, Kor artık yeniden yazmaz. */
export interface MemorySection {
  name: string
  start: number
  end: number
  edited: boolean
}

export interface McpGenRequest {
  /** postgres://…, mysql://…, sqlite:dosya, OpenAPI/GraphQL adresi ya da dosyası, klasör. */
  source: string
  id?: string
  kind?: McpGenKind
  /** OpenAPI taban adresi / GraphQL uç noktası (şema dosyaları için). */
  baseUrl?: string
  allowWrites?: boolean
  /** bearer:SIR, header:AD:SIR, query:AD:SIR (SIR bir kasa adı). */
  auth?: string
  label?: string
}

export type McpGenKind = 'openapi' | 'graphql' | 'database' | 'local'

export interface McpGenAdded {
  id: string
  kind: McpGenKind
  tools: number
  note: string | null
}

export interface McpTestReport {
  id: string
  connectMs: number
  listMs: number
  tools: string[]
  probe: string | null
  probeMs: number
  probeOk: boolean
  probeOutput: string
  error: string | null
}

export interface KorFolderStatus {
  enabled: boolean
  path: string
  items: KorFolderItem[]
  projects: string[]
  /** `Gelen/` içinde bekleyen dosyalar */
  inbox: number
  /** `Görevler/` içinde bekleyen görevler */
  tasks: number
  mount: KorFolderMount | null
}

export interface KorFolderItem {
  name: string
  kind: 'link' | 'view' | 'dir'
  path: string
  target: string | null
}

export interface KorFolderMount {
  url: string
  /** Sürücü harfi (Windows), klasör (macOS) ya da GVfs yolu (Linux) */
  at: string
}

export interface KorFolderSync {
  written: string[]
  linked: string[]
  applied: string[]
  kept: string[]
  notes: string[]
}

export interface KorFolderRemove {
  removed: string[]
  kept: string[]
}

export interface KorFolderPass {
  sync: KorFolderSync
  tasks: KorFolderTask[]
  inbox: number
}

export type KorFolderTask = { status: 'done'; result: string; file: string } | { status: 'failed'; error: string; file: string }

export interface IntentRouteContext {
  /** Seçili dosya (eklenmiş tek dosya) */
  selection?: string
  tabUrl?: string
}

export type IntentDecision = { kind: 'agent' } | { kind: 'run'; plan: IntentPlan } | { kind: 'confirm'; plan: IntentPlan } | { kind: 'missing'; plan: IntentPlan } | { kind: 'choose'; options: IntentPlan[] } | { kind: 'pipeline'; steps: IntentStep[] } | {
  kind: 'schedule'
  prompt: string
  interval_minutes: number
  first_run: number
  plan: IntentPlan | null
}

export interface IntentPlan {
  /** Katalog anahtarı: `kor explain` */
  command: string
  /** `kor` sonrası argümanlar */
  argv: string[]
  /** `kor explain src/main.rs` */
  line: string
  slash: string
  desktop: string | null
  /** parity.toml açıklaması (Türkçe) */
  name: string
  run: IntentRun
  via: IntentVia
  score: number
  params: Record<string, string>
  missing: string[]
  /** Katalog fiyatından tahmin (her zaman ≈) */
  usd: number | null
  eta_s: number | null
}

export type IntentRun = 'auto' | 'confirm' | 'never'

export type IntentVia = 'keyword' | 'learned' | 'embedding' | 'model'

export type IntentStep = { type: 'command'; value: IntentPlan } | { type: 'agent'; value: string }

export interface IntentRunResult {
  code: number | null
  output: string
  line: string
}

export interface IntentEntry {
  command: string
  run: IntentRun
  name: string
  tui: string | null
  desktop: string | null
  phrases: IntentPhrase[]
}

export interface IntentPhrase {
  lang: string
  text: string
}

export interface IntentPattern {
  text: string
  command: string
  at: number
}

export interface StatusView {
  status: GraphStatus
  packages: number
  /** Ajanın bu oturumda okuduğu/değiştirdiği kök-göreli dosyalar */
  touched?: string[]
}

export interface GraphStatus {
  root: string
  files: number
  nodes: number
  edges: number
  communities: number
  /** Unix saniye */
  last_update: number | null
  languages: Record<string, number>
  nodes_by_kind: Record<string, number>
  edges_by_kind: Record<string, number>
}

export interface Snapshot {
  nodes: SnapshotNode[]
  edges: SnapshotEdge[]
  /** Filtreye uyan toplam düğüm (kesilmeden önce) */
  total: number
  truncated: boolean
  communities: Community[]
}

export interface SnapshotNode {
  id: string
  kind: NodeKind
  name: string
  file: string | null
  line: number | null
  community: number
  degree: number
  churn: number
}

export type NodeKind = 'function' | 'struct' | 'trait' | 'module' | 'table' | 'env_var' | 'config' | 'doc' | 'decision' | 'package' | 'server' | 'service'

/** `[kaynak dizini, hedef dizini, tür, birlikte değişme oranı | null]` */
export type SnapshotEdge = [number, number, EdgeKindName, number | null]

export type EdgeKindName = 'calls' | 'imports' | 'implements' | 'extends' | 'contains' | 'reads_table' | 'writes_table' | 'reads_env' | 'co_changes' | 'documented_by' | 'decided_by' | 'depends_on' | 'references' | 'runs_on'

export interface Community {
  id: number
  label: string
  size: number
}

export interface Explanation {
  node: GraphNode
  source: string | null
  community: Community | null
  callers: Link[]
  callees: Link[]
  co_changes: Link[]
  decisions: NodeRef[]
  alternatives: NodeRef[]
}

export interface GraphNode {
  id: string
  kind: NodeKind
  name: string
  file: string | null
  span: [number, number] | null
  community: number
  degree: number
  churn_30d: number
  text?: string
}

export interface Link {
  node: NodeRef
  kind: EdgeKind
  confidence: Confidence
}

export interface NodeRef {
  id: string
  kind: NodeKind
  name: string
  file: string | null
  line: number | null
}

/** serde `tag = "kind"`: `{ kind: 'co_changes', ratio }` ya da `{ kind: 'calls' }` */
export interface EdgeKind {
  kind: EdgeKindName
  ratio?: number
}

export interface Confidence {
  level: 'extracted' | 'inferred' | 'ambiguous'
  score?: number
}

export interface PathStep {
  node: NodeRef
  edge: EdgeKind | null
  forward: boolean
}

export interface QueryHit {
  node: NodeRef
  score: number
  matched: string[]
}

export interface Hotspot {
  node: NodeRef
  degree: number
  churn_30d: number
  score: number
}

export interface GraphDiff {
  rev: string
  added_nodes: NodeRef[]
  removed_nodes: NodeRef[]
  added_edges: { source: string; target: string; kind: string }[]
  removed_edges: { source: string; target: string; kind: string }[]
}

export interface Impact {
  roots: NodeRef[]
  affected: { node: NodeRef; depth: number; via: EdgeKind }[]
  files: string[]
  truncated: boolean
}

export type ExportFormat__graph = 'json' | 'mermaid' | 'obsidian' | 'html' | 'graphify'

export interface ExportReport {
  path: string
  files_written: number
  nodes_omitted: number
}

export interface UpdateStats {
  scanned: number
  parsed: number
  cached: number
  removed: number
  changed: boolean
  nodes: number
  edges: number
  communities: number
  elapsed_ms: number
}

export interface GraphUpdatedEvent {
  root: string
  stats: UpdateStats
  status?: GraphStatus | null
}

export interface GraphTouchedEvent {
  root: string
  files: string[]
}

export interface ForgeDetect {
  remote: ForgeRemote | null
  error: string | null
  branch: string | null
  base: string
  hasUpstream: boolean
}

export interface ForgeRemote {
  kind: ForgeKind
  host: string
  owner: string
  repo: string
  web: string
  api: string
}

export type ForgeKind = 'github' | 'gitlab' | 'gitea' | 'bitbucket'

export interface PersonaInfo {
  name: string
  source: 'builtin' | 'user' | 'project'
  title: string
}

export interface PrDescription {
  title: string
  body: string
  head: string
  base: string
  firstPush: boolean
  note: string | null
}

export interface PrRef {
  url: string
  number: number
  forge: ForgeKind
}

export interface PrInfo {
  number: number
  title: string
  url: string
  head: string
  base: string
  author: string
  state: string
  draft: boolean
}

export interface ReviewResult {
  report: ReviewReport
  markdown: string
  number?: number
}

export interface ReviewReport {
  persona: string
  findings: Finding__forge[]
  model_error: string | null
}

export interface Finding__forge {
  file: string
  line: number | null
  severity: Severity__forge
  rule: string
  message: string
  source: 'check' | 'model'
}

export type Severity__forge = 'info' | 'low' | 'medium' | 'high' | 'critical'

export interface RespondDraft {
  pr: PrInfo
  comments: PrComment[]
  replies: { id: string; body: string }[]
  edits: { path: string; before: string | null; after: string | null }[]
}

export interface PrComment {
  id: string
  author: string
  body: string
  path: string | null
  line: number | null
}

export interface ReviewOptions {
  persona?: string | null
  staged?: boolean
  range?: string | null
  pr?: string | number | null
  model?: string | null
}

export interface FindResult {
  found: Found
  explanation: string | null
  provider: string | null
  explainError: string | null
}

export interface Found {
  question: string
  hits: FindHit[]
  links: FindLink[]
  text_index: 'semantic' | 'keyword'
  graph_nodes: number
  notes: string[]
}

export interface FindHit {
  file: string
  line: number
  end: number
  name: string | null
  kind: NodeKind | null
  /** Graf düğüm kimliği (grafta açmak için) */
  node: string | null
  sources: FindSource[]
  score: number
}

export type FindSource = 'graph' | 'semantic' | 'keyword'

export interface FindLink {
  from: string
  from_at: string | null
  kind: string
  to: string
  to_at: string | null
}

export interface EnvStatus {
  enabled: boolean
  onMismatch: 'stop' | 'warn'
  detected: {
    files: string[]
    kind: EnvKind | null
    pins: { tool: EnvTool; version: string; source: string }[]
    dockerfile_dev: boolean
  }
  wrap: boolean
  toolMissing: boolean
  reason: string | null
  example: string | null
  sandbox: 'inside' | 'asks' | 'image' | 'none'
  sandboxBackend: string
  versions: EnvVersion[]
  problems: string | null
}

export type EnvKind = { kind: 'devcontainer' } | { kind: 'compose'; file: string; service: string | null } | { kind: 'nix'; flake: boolean } | { kind: 'mise' } | { kind: 'asdf' }

export type EnvTool = 'node' | 'rust' | 'python' | 'go'

export interface EnvVersion {
  tool: EnvTool
  expected: string
  source: string
  found: string | null
  /** null: karşılaştırılamaz (lts/*, stable) */
  ok: boolean | null
}

export interface DepsList {
  packages: DepPackage[]
  sources: string[]
  /** [dosya, hata] */
  errors: [string, string][]
}

export interface DepPackage {
  /** OSV ekosistem adı: crates.io, npm, PyPI, Go, Packagist, NuGet, RubyGems, Maven */
  ecosystem: string
  name: string
  /** Kilit dosyasında sabitlenmemişse boş */
  version: string
  direct: boolean
  dev: boolean
  /** Kaynağın bildirdiği haliyle; yoksa bilinmiyor (tahmin edilmez) */
  license?: string
  /** `lockfile` ya da kurulu paketin manifest yolu */
  license_from?: string
  /** Doğrudan bağımlılıkta manifest, dolaylıda kilit dosyası */
  file: string
  line: number
  /** Dolaylı paketi çeken doğrudan bağımlılıklar */
  via?: string[]
  class: LicenseClass
}

export type LicenseClass = 'permissive' | 'weak-copyleft' | 'strong-copyleft' | 'proprietary' | 'unknown'

export interface DeployEnvs {
  envs: string[]
  state: { envs: Record<string, EnvState> }
}

export interface EnvState {
  commit: string | null
  at: number
  services: string[]
  color: string | null
  last: DeployReport | null
}

export interface DeployReport {
  env: string
  ok: boolean
  cancelled: boolean
  commit: string | null
  services: string[]
  health: HealthResult[]
  rolled_back: boolean
  color: string | null
  at: number
  log: string[]
}

export interface HealthResult {
  service: string
  ok: boolean
  detail: string
}

export interface DeployPlan {
  env: string
  target: 'compose' | 'kubernetes' | 'dockerfile' | 'none'
  head: string | null
  since: string | null
  dirty: boolean
  /** null: kayıtlı dağıtım yok, her şey değişmiş sayılır */
  changed_files: string[] | null
  services: string[]
  all_services: string[]
  steps: DeployStep[]
  rollback_steps: DeployStep[]
  rollback: 'auto' | 'ask' | 'off'
  health_timeout: number
  color: string | null
  notes: string[]
}

export interface DeployStep {
  id: number
  kind: StepKind__deploy
  program: string
  args: string[]
  env: [string, string][]
  display: string
  /** Bir şeyi değiştirir: kullanıcı onayı gerekir */
  approve: boolean
  destructive: boolean
  service: string | null
}

export type StepKind__deploy = 'snapshot' | 'build' | 'pull' | 'up' | 'apply' | 'health' | 'http_health' | 'switch' | 'stop' | 'rollback'

export interface RiskReport {
  env: string
  since: string | null
  risks: Risk__deploy[]
}

export interface Risk__deploy {
  level: 'high' | 'medium' | 'info'
  kind: string
  title: string
  file: string | null
  detail: string
}

export type DeployEvent = { type: 'start'; step: DeployStep } | { type: 'output'; id: number; line: string } | { type: 'done'; id: number; ok: boolean; detail: string } | { type: 'skipped'; id: number } | { type: 'note'; text: string }

export interface Watcher {
  id: number
  source: string
}

export interface Incident {
  id: string
  source: string
  title: string
  level: string | null
  count: number
  first_at: number
  last_at: number
  /** Log satırları: güvenilmeyen veri */
  context: string[]
  fix: Fix | null
}

export interface Fix {
  branch: string
  base: string
  pr: string | null
}

export type WatchEvent = { type: 'listening'; url: string } | { type: 'incident'; incident: Incident } | { type: 'repeat'; id: string; count: number } | { type: 'limited'; total: number } | { type: 'ended'; error: string | null }

export interface IndexSuggestion {
  connection: string
  table: string
  columns: string[]
  reason: string
  sql: string
}

export interface MigrationStyle {
  framework: string
  dir: string
  generator: string | null
  examples: string[]
}

export interface CostReport__cost {
  days: number
  rows: CostRow[]
  total: CostRow
  today: CostRow
  month: CostRow
  /** Ay sonu projeksiyonu, her zaman tahmin */
  projection_usd: number
  daily_limit_usd: number
  monthly_limit_usd: number
  /** Görsel, video ve ses üretimleri (ADR 0060); toplamın parçası */
  media?: CostRow
}

export interface RoutingDecision {
  id: string
  at: number
  category?: RouteCategory
  model?: string
  reason?: string
  task?: string
  outcome?: boolean
}

export type RouteCategory = 'simple_edit' | 'complex_refactor' | 'test_generation' | 'docs' | 'review' | 'planning'

export interface QuotaReport {
  keys: QuotaKey[]
  last_24h: { provider: string; requests: number; prompt: number; completion: number }[]
  failover: string[]
  /** [toplam kredi, kullanılan] USD */
  openrouter: [number, number] | null
  notes: string[]
}

export interface QuotaKey {
  provider: string
  key: string
  remaining_requests: number | null
  limit_requests: number | null
  remaining_tokens: number | null
  limit_tokens: number | null
  reset: string | null
  used_percent: number | null
  low: boolean
  seen: string
}

export interface BudgetConfig {
  daily_limit_usd: number
  monthly_limit_usd: number
  alert_at_percent: number
  on_limit: OnLimit
}

export type OnLimit = 'downgrade' | 'pause' | 'ask'

export type RoutingConfig = {
  simple_edit: string
  complex_refactor: string
  test_generation: string
  docs: string
  review: string
  planning: string
} & {
  fallback: string[]
  enabled: boolean
  verify_with: string
  consensus: string[]
  shadow: string
  shadow_threshold: number
  shadow_min_samples: number
  repair: string
}

export interface ContextLast {
  breakdown: ContextBreakdown__cost | null
  total: number | null
  text: string | null
  warning: string | null
  thresholds: ContextThresholds
}

export interface ContextBreakdown__cost {
  /** Unix saniye */
  ts: number
  model: string
  window: number
  estimated: boolean
  system: number
  rules: number
  repo_map: number
  memory: number
  tools: number
  active_files: number
  history: number
  tool_results: number
  warning: string | null
}

export interface ContextThresholds {
  warn_percent: number
  auto_summarize_percent: number
  summarize_model: string
  repo_map_tokens: number
  log: boolean
}

export interface BenchOptions {
  file?: string
  models?: string[]
  graph?: 'on' | 'off' | 'both'
  maxCost?: number
  confirm?: boolean
}

export interface BenchEstimate {
  jobs: number
  usd: number
  estimated: true
  /** Fiyatı bilinmeyen uzak modeller */
  unknown: string[]
  maxCost: number
  withinBudget: boolean
}

export interface BenchDone {
  cancelled: boolean
  report: BenchReport | null
}

export interface BenchReport {
  rows: BenchRow[]
  summary: BenchSummary[]
  routing: Record<string, string>
  routing_toml: string
  comparison: {
    baseline: BenchSummary
    routed: BenchSummary
    cost_delta_pct: number | null
    pass_delta_pp: number
    meets_acceptance: boolean | null
  } | null
  graph: BenchGraphComparison | null
}

export interface BenchRow {
  task: string
  category: string
  model: string
  arm: BenchArm
  passed: boolean
  wall_ms: number
  input_tokens: number
  output_tokens: number
  tokens_estimated: boolean
  cost_usd: number | null
  cost_estimated: boolean
  tool_mode: string
  tool_calls_ok: number
  tool_calls_repaired: number
  tool_calls_failed: number
  error: string | null
}

export type BenchArm = 'race' | 'baseline' | 'routed' | 'graph-off' | 'graph-on'

export interface BenchSummary {
  model: string
  arm: BenchArm | null
  tasks: number
  passed: number
  pass_rate: number
  wall_ms: number
  input_tokens: number
  output_tokens: number
  cost_usd: number | null
  cost_estimated: boolean
  tool_calls_ok: number
  tool_calls_repaired: number
  tool_calls_failed: number
  tool_success_rate: number | null
  emulated: number
}

export interface BenchGraphComparison {
  off: BenchSummary
  on: BenchSummary
  token_delta_pct: number | null
  cost_delta_pct: number | null
  pass_delta_pp: number
  meets_acceptance: boolean | null
}

export type BenchProgress = { kind: 'start'; index: number; total: number; task: string; model: string; arm: BenchArm } | { kind: 'done'; index: number; total: number; row: BenchRow }

export interface CompatConflict {
  /** Aynı kural: aynı glob kümesi ya da aynı dosya adı */
  key: { kind: 'globs'; globs: string[] } | { kind: 'name'; name: string }
  /** Birleştirme sırasında (en düşük öncelik önce) */
  rules: CompatSource[]
  /** Kazanan kaynağın yolu (birleştirmede en son okunur) */
  winner: string
}

export interface CompatSource {
  /** Aracın kısa adı (kor, claude, codex, cursor, windsurf, ...) */
  eco: string
  scope: 'user' | 'project' | 'local'
  path: string
}

export type CompatExportTarget = 'cursor' | 'claude' | 'agents'

export interface BatchPlan {
  id: string
  rows: BatchRow[]
  /** Denetimde düşen işlemler ve nedenleri */
  warnings: string[]
}

export interface BatchRow {
  op: BatchOp
  /** Kaynak dosya(lar), virgülle ayrılmış mutlak yollar */
  from: string
  /** Hedef (yeni dosya, yeni ad ya da yerinde değişen dosya) */
  to: string
  detail: string
}

export type BatchOp = 'rename' | 'convert_image' | 'strip_metadata' | 'replace' | 'reencode' | 'csv_clean' | 'pdf_merge' | 'media'

export interface BatchApplied {
  applied: number
  failed: { row: BatchRow; error: string }[]
  /** `kor undo --session <id>` */
  undo: string | null
}

export interface ProviderTestResult {
  check: ProviderCheck
  model: string | null
  stream: {
    ok: boolean
    firstTokenMs: number | null
    totalMs: number | null
    text: string
    error: ProviderErrorView | null
  } | null
}

export interface ProviderCheck {
  provider: string
  ok: boolean
  latencyMs: number | null
  models: number | null
  error: ProviderErrorView | null
}

export interface ProviderErrorView {
  kind: string | null
  status: number | null
  message: string
}

/** `kor provider status` satırı: son çağrılardan gecikme, hata oranı ve kota. */
export interface ProviderStatusRow {
  provider: string
  latencyMs: number | null
  errorRate: number | null
  calls: number
  remainingRequests: number | null
  remainingTokens: number | null
  limitRequests: number | null
  limitTokens: number | null
  reset: string | null
  lastError: string | null
  updatedAt: number | null
}

export interface CatalogResult {
  origin: 'published' | 'openrouter'
  /** Unix saniye */
  fetchedAt: number
  fromCache: boolean
  notices: string[]
  models: CatalogEntry__providers[]
}

/** `kor_providers::model_catalog::ModelData` (§4.4 şeması, alan adları olduğu gibi). */
export interface CatalogEntry__providers {
  model_id: string
  provider: string | null
  aa_intelligence_index: number | null
  aa_coding_index: number | null
  aa_math_index: number | null
  speed_tps: number | null
  ttft_ms: number | null
  input_price_per_m: number | null
  output_price_per_m: number | null
  context: number | null
  tool_calling: boolean | null
  vision: boolean | null
  hf_leaderboard_avg: number | null
  license: string | null
  updated_at: string | null
  /** CI'ın bu modelin AA verisini son aldığı an (AA düştüğünde eski değer taşınır) */
  aa_fetched: string | null
  source: Record<string, string>
}

export interface ProxyStart {
  running: boolean
  url: string
  panel?: string
  key?: string | null
  noAuth?: boolean
  privacy?: boolean
  warnings?: string[]
  notes?: string[]
}

export interface ProxyStatus {
  running: boolean
  url?: string
  usage?: {
    total_requests: number
    success_count: number
    failure_count: number
    total_tokens: number
    apis: Record<string, { total_requests: number; total_tokens: number; models: Record<string, ProxyModelUsage> }>
  }
  credentials?: ProxyCredential[]
}

export interface ProxyModelUsage {
  total_requests: number
  success_count: number
  failure_count: number
  rate_limited: number
  input_tokens: number
  output_tokens: number
  total_tokens: number
  last_used: number | null
}

export interface ProxyCredential {
  id: string
  provider: string
  key: string
  base_url: string
  weight: number
  priority: number
  cooling_for_secs: number
  success: number
  failed: number
}

export interface ProxyConfig {
  path: string | null
  exists: boolean
  running: boolean
  config: Record<string, unknown> & { host?: string | null; port?: number }
}

export interface ProxyOauthAccount {
  file_name: string
  provider: string
  label: string
  pool_id: string
  weight: number
  priority: number
  in_pool: boolean
}

export interface ProxyRouting {
  strategy: 'round-robin' | 'weighted-round-robin' | 'fill-first'
  requestRetry: number
}

export interface ProxyOauthAccounts {
  dir: string
  accounts: ProxyOauthAccount[]
}

export interface ProxyOauthLoginResult {
  provider: string
  label: string
  fileName: string
  path: string
  dir: string
}

export interface ComponentView {
  name: string
  version: string
  about: string
  /** Bu makine için indirme boyutu (bayt); yayımlanmış derleme yoksa null */
  size: number | null
  installed: boolean
  path: string | null
  sha256: string | null
}

export interface MediaJob {
  jobId: string
  kind: 'fal' | 'hf' | 'gen' | 'collect'
  target: string
  startedAt: number
}

export interface HfModel {
  id: string
  downloads: number | null
  likes: number | null
  pipeline: string | null
  library: string | null
}

export interface HfInfo {
  id: string
  pipeline: string | null
  library: string | null
  license: string | null
  downloads: number | null
  likes: number | null
  gated: boolean
  files: { path: string; size: number | null; sha256: string | null }[]
}

export interface MediaProgress {
  jobId: string
  model: string
  status: string
}

export interface HfProgress {
  jobId: string
  repo: string
  file: string
  done: number
  total: number
}

/** Paketle gelen kor CLI'ın PATH durumu (ADR 0032) */
export interface CliStatus {
  /** Bu kurulumda paketlenmiş kor var mı (geliştirme derlemesinde yok) */
  available: boolean
  installed: boolean
  /** Kor'un yaptığı değişiklik var ve geri alınabilir */
  removable: boolean
  /** .deb/.rpm: kor zaten /usr/bin'de */
  byPackage: boolean
  /** PATH'e eklenen klasör (Windows) ya da bağlantı (macOS/Linux) */
  location: string | null
}

/** Kuralın ne zaman uygulandığı (core.compat `applies`) */
export type CompatApplies = { kind: 'always' } | { kind: 'globs'; globs: string[] } | { kind: 'on_request'; description: string } | { kind: 'manual' }

/** `question` olayı (§28.13): `default` önerilen seçenek (0'dan). */
export interface QuestionEvent {
  type: 'question'
  sessionId: string
  id: string
  question: string
  options: string[]
  default: number
}

export interface CompactItem {
  index: number
  kind: 'user' | 'assistant' | 'tool'
  label: string
  tokens: number
}

/** `compactPreview` olayı ve `kor:context/compactPreview` (§28.1). */
export interface CompactPreviewEvent {
  type: 'compactPreview'
  sessionId: string
  id: string
  items: CompactItem[]
  /** false: yalnızca bilgi (`context.compact_confirm = false`) */
  confirm: boolean
}

/** `kor:runtimeProgress` (bileşen indirme ilerlemesi; `id` = bileşen adı). */
export interface RuntimeTick {
  id: string
  phase: string
  percent: number | null
  line: string
}

export interface ReplayFile {
  path: string
  before: string | null
  after: string | null
  truncated?: boolean
}

/** Dosya başına karar; yazılmayan dosyalar `approval` yanıtını izler. */
export type FileDecision = 'accept' | 'reject' | { edit: string }

export type FileDecisions = Record<string, FileDecision>

/** `confirm` olayının ipc.ts'de tiplenmemiş alanları (ipc.ts paylaşımlı, burada okunur). */
export interface ConfirmReviewFields {
  files?: ReviewFile[]
  destructive?: string | null
  /** §28.3: düzenlemenin kural uyumu (kırmızı/sarı/yeşil), kaynak dosya:satır ile. */
  rules?: RuleFinding[]
}

/** §28.3 kural uyum bulgusu (ADR 0063). */
export interface RuleFinding {
  verdict: 'applied' | 'relevant' | 'violated'
  rule: string
  source: string
  line: number
  file: string | null
  fileLine: number | null
  message: string
  deterministic: boolean
}

export type PrereqId = 'docker' | 'podman' | 'wsl2' | 'xcode-clt' | 'webview2' | 'accessibility' | 'screen-recording' | 'microphone'

export interface PrereqRow {
  id: PrereqId
  /** null: yalnızca işletim sistemi bilir (izinler; ilk ihtiyaçta sorulur) */
  installed: boolean | null
  permission: boolean
  /** Masaüstünden çalışacak tam komut (yükseltme sarmalayıcısıyla) */
  command: string | null
  manager: string | null
  elevation: 'none' | 'uac' | 'sudo' | null
  action: 'install' | 'open-settings' | null
  reason: 'no-manager' | 'not-needed' | null
}

export interface PrereqInstallResult {
  ok: boolean
  /** Çıkış kodu; null ve ok=false: kullanıcı onaylamadı */
  code: number | null
  row: PrereqRow
}

export interface PrereqProgress {
  id: PrereqId
  line: string
}

/** `catalog.hooks`: one hook from hooks.toml / Claude Code settings (`kor hooks list`). */
export interface HookRow {
  event: string
  korEvents: string[]
  matcher: string | null
  command: string
  source: { eco: string; scope: string; path: string }
}

export interface DiscoveredProvider {
  provider: string
  name: string
  baseUrl: string
  models: string[]
  moved: boolean
}

export interface FalModel {
  id: string
  name: string | null
  category: string | null
  description: string | null
}

/** Sağlayıcının liste fiyatı. */
export interface MediaPrice {
  amount: number
  currency: string
  unit: string
}

export interface MissingInput {
  name: string
  description: string | null
  options: string[]
  default: unknown
}

/** §20.2: either the required inputs still missing, or the saved outputs. */
export type MediaRunResult = { missing: MissingInput[] } | { jobId: string; paths: string[]; seconds: number; report: string }

export interface SlowQuery {
  query: string
  calls: number
  mean_us: number
  total_us: number
}

export interface LiveTable {
  name: string
  columns: string[]
  references?: string[]
  indexes?: string[]
  fk_columns?: string[]
}

export interface DbSnapshot {
  connection: string
  kind: string
  taken_at: number
  tables: LiveTable[]
  slow_queries?: SlowQuery[]
}

/** One table with its schema and the code touching it (`kor db show`). */
export interface TableInfo {
  node: NodeRef
  columns: string[]
  references: NodeRef[]
  referenced_by: NodeRef[]
  readers: Link[]
  writers: Link[]
  live: string[]
  indexes: string[]
  slow_queries: SlowQuery[]
}

/** §28.1: the parts of a session context, pinned indexes and the context window. */
export interface ContextItems {
  items: CompactItem[]
  pinned: number[]
  window: number
}

/** `agent.status` (§16.2): verb, elapsed time and tokens chosen in the core (`kor_agent::remote::wire::event_json`). */
export interface AgentStatusBody {
  type: 'status'
  verb: string
  verbIndex: number
  elapsedMs: number
  tokens: number
}

export interface AgentStatusEvent extends AgentStatusBody {
  sessionId: string
}

export interface QuestionEvent__body {
  type: 'question'
  id: string
  question: string
  options: string[]
  default: number
}

export interface CompactPreviewEvent__body {
  type: 'compactPreview'
  id: string
  items: CompactItem[]
  /** false: yalnızca bilgi (`context.compact_confirm = false`) */
  confirm: boolean
}

/** IPC trace of this app (§17.2): the NDJSON file every call, result and event goes to. */
export interface IpcTraceState {
  enabled: boolean
  path: string | null
}

export interface PerfmonStat {
  p50: number
  p95: number
  n: number
}

export interface PerfmonCause {
  source: string
  kind: string
  what: string
  count: number
  p95_ms: number
  max_ms: number
  total_ms: number
}

/** Medya yeteneği (§20.1, kor_providers::media_catalog::Capability). */
export type MediaCapability = 'image' | 'edit' | 'upscale' | 'remove-bg' | 'video' | 'image-to-video' | 'tts' | 'asr' | 'audio' | 'music'

/** Bir modelin girdisi: tür, seçenekler, aralık, varsayılan. */
export interface MediaCatalogParam {
  name: string
  kind: 'string' | 'integer' | 'number' | 'boolean' | 'enum' | 'file' | 'array' | 'object'
  role: string
  required: boolean
  options?: unknown[]
  min?: number
  max?: number
  default?: unknown
  description?: string
}

/** Sağlayıcıdan bağımsız üretim modeli (§20.1). */
export interface MediaCatalogModel {
  provider: string
  id: string
  name: string
  capabilities: MediaCapability[]
  params: MediaCatalogParam[]
  price: MediaPrice | null
  source: string
  acceptsReference: boolean
}

export interface MediaCatalog {
  fetched: number
  providers: string[]
  errors: string[]
  models: MediaCatalogModel[]
}

/** Üretim öncesi maliyet; `estimated` ≈ gösterilir. */
export interface MediaEstimate {
  amount: number
  currency: string
  estimated: boolean
}

export interface MediaPlanQuestion {
  name: string
  role: string
  kind: string
  description: string
  options: unknown[]
  min: number | null
  max: number | null
  default?: unknown
}

export interface MediaUnsupported {
  name: string
  asked: unknown
  nearest?: unknown
  allowed: string
}

/** §20.2 parametre kartının verisi. */
export interface MediaPlan {
  model: MediaCatalogModel
  plan: {
    input: object
    questions: MediaPlanQuestion[]
    unsupported: MediaUnsupported[]
    estimate: MediaEstimate | null
  }
  estimateText: string | null
  alternatives: { name: string; models: { model: MediaCatalogModel; estimate: MediaEstimate | null }[] }[]
  eta: number | null
  confirm: boolean
  confirmAbove: number
}

/** Galeri öğesi: `<dosya>.kor.json` yan dosyası (§20.3). */
export interface MediaGalleryItem {
  id: string
  file: string
  kind: 'image' | 'video' | 'audio' | 'other'
  capability: string | null
  provider: string
  model: string
  prompt: string
  params: unknown
  seed: number | null
  cost: number | null
  currency: string | null
  estimated: boolean
  secs: number
  created: number
  inputs: string[]
  parent: string | null
  action: string | null
  path?: string
}

export interface MediaGalleryDetail {
  item: MediaGalleryItem
  path: string
  chain: { item: MediaGalleryItem; path: string }[]
  children: MediaGalleryItem[]
}

/** Sağlayıcıda süren, henüz toplanmamış istek (§20.4). */
export interface MediaPendingJob {
  id: string
  provider: string
  model: string
  prompt: string
  capability?: string | null
  started: number
}

export interface MediaGenerateOptions {
  name?: string
  saveAs?: string
  action?: string
  derivedFrom?: string
}

export interface MediaGenerateResult {
  jobId: string
  paths: string[]
  items: MediaGalleryItem[]
  report: string
}

export interface ParamField {
  name: string
  description: string
  options: unknown[]
  default?: unknown
  min: number | null
  max: number | null
}

/** §20.2 ajan aracının parametre kartı (kor_tools::ParamCard). */
export interface ParamCard {
  title: string
  fields: ParamField[]
  notes: string[]
  cost: string | null
  eta: string | null
  confirm: boolean
  target: string | null
  fixed: object
}

/** `paramCard` olayı (§20.2): yanıt `prefs.answer(id, { values } | { cancel: true })`. */
export interface ParamCardEvent {
  type: 'paramCard'
  sessionId: string
  id: string
  card: ParamCard
}

/** Parametre kartının yanıtı (iptal: CompactAnswer `{ cancel: true }`). */
export interface ParamCardAnswer {
  values: object
}

export interface PeerGrant {
  dirs: string[]
  write: boolean
  commands: string[]
  tools: string[]
  memory: boolean
}

export interface PeerLoad {
  name: string
  cpus: number
  busy: number | null
  tasks: number
  version: string
}

export interface PeerRow {
  name: string
  fingerprint: string
  addrs: string[]
  ssh: string | null
  grant: PeerGrant
  load: PeerLoad | null
  score: number | null
  error: string | null
}

export interface PeerList {
  self: { name: string; fingerprint: string; listen: string }
  handoff: string | null
  serving: string | null
  leastBusy: string | null
  peers: PeerRow[]
}

export interface PeerPairCode {
  code: string
  fingerprint: string
  size: number
  modules: boolean[]
  expiresAt: number
}

export interface PeerTaskState {
  id: string
  state: string
  project?: string
  prompt: string
  started?: number
  updated?: number
  consumed?: number
  events?: number
}

export interface PeerResult {
  task: string
  state: string
  diff: string
  files: { path: string; kind: 'added' | 'deleted' | 'modified' }[]
}

export interface PeerApplied {
  path: string
  status: 'written' | 'deleted' | 'conflict' | 'failed'
  detail: string | null
}

export interface PeerSyncReport {
  local: { added: number; updated: number; removed: number }
  remoteApplied: boolean
  records: number
  conflicts: { key: string; task: string; field: string }[]
}

export interface PeerEventPayload {
  peer: string
  task: string | null
  event: RemoteWireEvent
}

export interface PeerHandoff {
  peer: string
  session: string
  task: string
}

export interface AcpSetup {
  command: string
  args: string[]
  protocolVersion: number
  zed: object
}

export interface AcpAgentRow {
  name: string
  command: string
  args: string[]
}

export interface AcpRunResult {
  stopReason: string
  text: string
}

export interface AcpUpdate {
  agent: string
  /** ACP session/update gövdesi */
  update: object
}

export interface AcpPermission {
  id: number
  agent: string
  toolCall: object
  options: object[]
}

export interface PrivacyCardText {
  training: string
  retention: string
  region: string
  zdr: string
  subProcessor: string
  law: string
}

export interface PrivacyCard {
  provider: string
  model: string | null
  training: 'no' | 'opt-out' | 'yes' | 'unknown'
  retention: 'indefinite' | 'unknown' | { days: number }
  region: 'eu' | 'us' | 'tr' | 'cn' | 'other' | 'local' | 'unknown'
  euResidency: boolean | null
  zdr: 'required' | 'not-needed' | 'unknown'
  gateway: boolean
  subProcessor: string | null
  notes: string[]
  sources: { url: string; retrieved: string; quote: string }[]
  local: boolean
}

export interface PrivacyRow {
  card: PrivacyCard
  unmet: string[]
  text: PrivacyCardText
}

export interface PrivacyCards {
  require: string[]
  providers: PrivacyRow[]
}

/** kor chrome allow|deny alan adı listeleri. */
export interface ChromeDomains {
  allow: string[]
  deny: string[]
}

export interface ComputerStatus {
  enabled: boolean
  front: { app: string; tier: ComputerTier; class: ComputerClass } | null
  watching: boolean
  paused: 'mouse' | 'input' | null
  activity: ComputerActivity
  apps: { app: string; tier: ComputerTier; source: 'config' | 'default'; class: ComputerClass }[]
  classes: { class: ComputerClass; tier: ComputerTier }[]
}

export type ComputerTier = 'blocked' | 'read' | 'click' | 'ask' | 'full'

export type ComputerClass = 'kor' | 'terminal' | 'password_manager' | 'banking' | 'browser' | 'ide' | 'other'

export interface ComputerActivity {
  active: boolean
  tool: string
  app: string | null
  paused: 'mouse' | 'input' | null
  at: number
}

export interface WindowLayout {
  name: string
  saved_at: number
  monitors: LayoutMonitor[]
  windows: LayoutSaved[]
}

export interface LayoutMonitor {
  name: string
  x: number
  y: number
  width: number
  height: number
  primary: boolean
}

export interface LayoutSaved {
  process: string
  title: string
  monitor: string
  rx: number
  ry: number
  rw: number
  rh: number
  state: string
}

export interface LayoutPlan {
  moves: LayoutMove[]
  missing: string[]
}

export interface LayoutMove {
  hwnd: number
  process: string
  title: string
  x: number
  y: number
  w: number
  h: number
  state: string
}

export interface CleanOutcome {
  id: string
  kind: string
  path: string | null
  freed: number
  error: string | null
}

export interface CleanCandidate {
  id: string
  kind: string
  path: string | null
  bytes: number
  approx: boolean
}

export interface HealthAlert {
  id: string
  kind: string
  level: HealthLevel
  title: string
  body: string
  clean: boolean
}

export type HealthLevel = 'warn' | 'critical'

export interface SystemProfile {
  collected_at: number
  os: {
    family: string
    name: string
    version: string
    long_name: string
    kernel: string
    arch: string
    hostname: string
    distro: SystemDistro | null
    package_managers: string[]
    desktop: string | null
  }
  cpu: { model: string; cores: number | null; threads: number }
  memory: { total_mb: number; available_mb: number; swap_total_mb: number; swap_used_mb: number }
  gpus: SystemGpu[]
  disks: SystemDisk[]
  battery: SystemBattery | null
  network: { name: string; addrs: string[] }[]
  toolchains: Record<string, string>
  shell: string | null
  terminal: string | null
  displays: SystemDisplay[]
  keyboard: SystemKeyboard | null
}

export interface SystemDistro {
  id: string
  family: string
  name: string
  version: string
  immutable: boolean
}

export interface SystemGpu {
  name: string
  vram_mb: number | null
}

export interface SystemDisk {
  mount: string
  name: string
  fs: string
  total_mb: number
  free_mb: number
  removable: boolean
}

export interface SystemBattery {
  percent: number
  charging: boolean
}

export interface SystemDisplay {
  name: string
  x: number
  y: number
  width: number
  height: number
  scale: number
  primary: boolean
}

export interface SystemKeyboard {
  raw: string
  lang: string | null
  variant: string | null
}

export interface SystemRefresh {
  profile: SystemProfile
  changed: string[]
}

/** Every bridge method, event and namespace (`window.kor`). */
export interface KorIpcApi {
  abort(sessionId: string): Promise<void>
  /** Pencere odaklanınca: bellekteki sohbeti yeniden etkin yapar; durum değiştiyse true */
  activateSession(id: string): Promise<boolean>
  approve(id: string, approval: Approval): Promise<void>
  /** Arka planda üretir/günceller; ilerleme getProjectMap ile izlenir. force: tüm alanlar baştan */
  buildProjectMap(force: boolean): Promise<void>
  cancelProjectMap(): Promise<void>
  /** Klasörü kaldırır; sohbet varsayılan çalışma alanında devam eder. */
  clearWorkspace(): Promise<void>
  /** Eski turları özetler (/compact); sohbet çalışırken reddedilir */
  compactSession(sessionId: string): Promise<CompactionResult>
  createSection(name: string): Promise<Section>
  deleteSection(id: string): Promise<void>
  deleteSession(id: string): Promise<void>
  /** [cdx] Sohbeti kopyalar (spinOff: 'dal' olarak adlandırır); yeni kimliği döner */
  duplicateSession(id: string, spinOff: boolean): Promise<string>
  /** Sıkıştırma özetini kullanıcı düzeltmesiyle değiştirir */
  editCompactionSummary(sessionId: string, body: string): Promise<boolean>
  /** [cdx] Sohbeti Markdown dosyasına dışa aktarır (kaydetme penceresi); iptalde null, yoksa yazılan yol */
  exportSession(id: string): Promise<string | null>
  /** Kaydı kimliğiyle siler (s/k/g önekli) */
  g11Forget(sessionId: string, id: string): Promise<void>
  /** Oturum/proje/global hafıza katmanları, olası çakışmalar ve güven puanı ayarı */
  g11GetOverview(sessionId: string): Promise<MemoryOverview>
  /** İki kaydın ikisi de geçerli: çift bir daha çakışma sayılmaz */
  g11KeepBoth(a: string, b: string): Promise<void>
  g11SetConfidence(enabled: boolean): Promise<void>
  g12AnswerBulkDelete(id: string, approved: boolean): Promise<void>
  g12GetSandbox(): Promise<SandboxStatus>
  /** Sohbet henüz başlamadıysa null */
  g12GetToolScopes(sessionId: string): Promise<G12ToolScope[] | null>
  /** Kullanıcı ayar dosyasına yazar; güncel durumu döner */
  g12SetSandbox(enabled: boolean): Promise<SandboxStatus>
  g12SetToolScope(sessionId: string, tool: string, level: G12ToolLevel): Promise<G12ToolScope[]>
  g16CliInstall(): Promise<CliStatus>
  g16CliRemove(): Promise<CliStatus>
  g16CliStatus(): Promise<CliStatus>
  /** Tüm ayarları bildirimsel dosyaya yazar; iptal edilirse null */
  g16ConfigExport(): Promise<string | null>
  /** Dosya seçilir, özet ana süreçte onaylatılır; iptal edilirse null */
  g16ConfigImport(): Promise<ConfigImportResult | null>
  /** Maskeli raporu kaydetme penceresiyle diske yazar; iptal edilirse null */
  g16CrashExport(): Promise<string | null>
  g16CrashList(): Promise<CrashInfo>
  g16CrashOpenFolder(): Promise<void>
  /** Kullanıcının tanımladığı uç noktaya ana süreçteki onaydan sonra gönderir; onaylanmazsa false */
  g16CrashSend(): Promise<boolean>
  /** Güncelleme denetler; bulunursa fark tabanlı (blockmap) indirir */
  g16UpdateCheck(): Promise<UpdateStatus>
  /** İndirilen güncellemeyi kurmak için uygulamayı yeniden başlatır */
  g16UpdateInstall(): Promise<void>
  g16UpdateStatus(): Promise<UpdateStatus>
  /** Kendine yeten HTML'i görünmez pencerede çizip PDF'e aktarır; çalışma dizinine kaydeder, iptalde null */
  g17ExportPdf(html: string, suggestedName: string): Promise<string | null>
  /** Çalışma dizinindeki .ipynb dosyasının ham metni */
  g20ReadNotebook(path: string): Promise<string>
  /** Ana süreçte yerel onaydan sonra proje Python'uyla çalıştırır; iptalde null */
  g20RunPython(code: string): Promise<G20PythonResult | null>
  /** Çalışma dizinindeki SQLite dosyasında salt okunur sorgu (onaylı); iptalde null */
  g20RunSql(dbPath: string, sql: string): Promise<G20SqlResult | null>
  /** TypeScript tiplerini siler (yalnız metin dönüşümü; kod çalışmaz) */
  g20StripTs(code: string): Promise<string>
  /** Belge önizlemesi için çalışma dizinindeki dosyanın ham baytları (hassas dosya reddedilir, boyut sınırlı) */
  g21ReadDoc(path: string): Promise<Uint8Array>
  /** Model başına araç modu; kullanıcı ayar dosyasına yazılır ('auto' kaydı siler) */
  g22SetToolMode(model: string, mode: ToolMode): Promise<void>
  /** Modelin yetenekleri, araç modu ve verilen eklerin nasıl gönderileceği (yerel / metne dönüştürülerek / desteklenmiyor) */
  g22Support(model: string, attachments: string[]): Promise<G22ModelSupport>
  g27InstallRuntime(id: string): Promise<{ installed: boolean; version: string | null; opened: boolean }>
  g27ListRuntimes(): Promise<RuntimeStatus[]>
  g2EditMessage(sessionId: string, payload: G2EditPayload): Promise<void>
  g2ModelCaps(): Promise<Record<string, G2ModelCaps>>
  /** Sürüklenen dosyanın yerel yolu (Electron webUtils, IPC değil); yol yoksa boş dize */
  g2PathForFile(file: File): string
  /** Kuyruğu boşaltır, çıkarılan kimlikleri döner */
  g2QueueClear(sessionId: string): Promise<string[]>
  /** Tur bittiğinde kuyrukta kalan mesaj varsa yeni tur başlatır */
  g2QueueFlush(sessionId: string): Promise<void>
  /** Henüz tüketilmediyse kuyruktan çıkarır; çıkarıldıysa true */
  g2QueueRemove(sessionId: string, queueId: string): Promise<boolean>
  g2Regenerate(sessionId: string, payload: G2RegeneratePayload): Promise<void>
  /** Sürüklenen mutlak yolları çalışma dizinine göre doğrular */
  g2ResolveDropped(absPaths: string[]): Promise<G2DropResult>
  /** Çalışma dizininde bulanık dosya/klasör araması (en fazla 50) */
  g2SearchFiles(query: string): Promise<G2FileHit[]>
  /** Boştaysa turu başlatır, çalışıyorsa kuyruğa alır (interrupt: turu kesip hemen başlatır) */
  g2Send(sessionId: string, payload: G2SendPayload): Promise<G2SendResult>
  g2StatFiles(paths: string[]): Promise<G2FileStat[]>
  /** Bekleyen ask_user / exit_plan_mode kartını cevaplar */
  g3Answer(sessionId: string, callId: string, answer: InteractAnswer): Promise<void>
  g3GetOutputStyle(): Promise<OutputStyle>
  /** Yalnızca genel (public) http(s) adresleri; null = önizlenecek bilgi yok */
  g3LinkPreview(url: string): Promise<LinkPreview | null>
  /** Yerel onay penceresinden sonra çalıştırır; iptalde null */
  g3RunCode(code: string, language: string): Promise<SnippetResult | null>
  /** Kaydetme penceresi açar; kaydedilen göreli yol veya iptalde null */
  g3SaveCode(content: string, language: string): Promise<string | null>
  g3SetOutputStyle(style: OutputStyle): Promise<void>
  g7AbortProbe(runId: string): Promise<void>
  g7AddPermissionRule(list: 'allow' | 'deny', rule: string): Promise<void>
  /** Ana süreçten Venice API'sine erişilebiliyor mu */
  g7CheckOnline(): Promise<boolean>
  /** mode 'trash': çöp kutusuna taşır, 'delete': kalıcı siler */
  g7ClearData(target: PrivacyClearTarget, mode: 'trash' | 'delete'): Promise<void>
  /** Venice modelleri + merkezi katalogdaki Artificial Analysis puanları (yerel kopya); force: önbellekleri yenile */
  g7GetCatalog(force: boolean): Promise<CatalogResponse>
  g7GetPermissionRules(): Promise<PermissionRulesInfo>
  g7GetPrivacyInfo(): Promise<PrivacyInfo>
  g7GetSettings(): Promise<G7Settings>
  /** Açık klasörün güven durumu; klasör yoksa null */
  g7GetTrust(): Promise<TrustInfo | null>
  g7GetUsageHistory(period: UsagePeriod): Promise<UsageHistory>
  g7OpenFolder(folder: PrivacyFolder): Promise<void>
  g7RemovePermissionRule(list: 'allow' | 'deny', rule: string): Promise<void>
  /** Araçsız tek tur test isteği; runId ile iptal edilir */
  g7RunProbe(runId: string, model: string, prompt: string): Promise<ProbeResult__model_probe>
  g7SetEditor(prefs: EditorPrefs): Promise<void>
  g7SetGit(prefs: GitPrefs): Promise<void>
  /** [perf] Denetim günlüğü saklama süresi */
  g7SetLogs(logs: { keepDays: number }): Promise<void>
  g7SetModelAliases(aliases: Record<string, string>): Promise<void>
  /** [cdx] Bildirim tercihleri (masaüstü/ses) */
  g7SetNotifications(prefs: NotificationPrefs): Promise<void>
  /** [cdx] Kullanıcı tanımlı ajan profilleri (tam liste yazılır) */
  g7SetProfiles(profiles: AgentProfile[]): Promise<void>
  g7SetScripts(scripts: ScriptPrefs): Promise<void>
  g7SetTaskModels(taskModels: TaskModels): Promise<void>
  g7SetTheme(theme: ThemeSetting): Promise<void>
  g7SetTrust(trusted: boolean): Promise<void>
  /** Otomatik sıkıştırma eşiği ayarları (tavan token ve oran) */
  getCompactSettings(): Promise<CompactSettings>
  /** sessionId null: yeni sohbet taslağı (yalnızca seçili modelin sınırı) */
  getContextInfo(sessionId: string | null): Promise<ContextView>
  /** [perf] Büyük dosya değişikliğinin tam içeriği (fileChanged.large geldiğinde isteğe bağlı) */
  getFileChange(sessionId: string, path: string): Promise<{ before: string | null; after: string } | null>
  /** Geri yüklenebilecek son açık sohbet (dosyası ve klasörü varsa) */
  getLastSession(): Promise<string | null>
  getMemory(): Promise<string>
  /** USD / 1M token, model listesi alındıktan sonra dolar */
  getModelPricing(): Promise<Record<string, { input: number; output: number }>>
  getProjectMap(): Promise<ProjectMapInfo>
  getSessionConfig(id: string): Promise<{ model: string; permissionMode: PermissionMode }>
  getState(): Promise<AppState>
  getSystemPrompt(): Promise<SystemPromptInfo>
  getUsage(sessionId: string | null): Promise<UsageInfo>
  /** /init: AGENTS.md taslağı; dosya varsa overwrite=false ile yalnızca {status:'exists'} döner */
  initProject(overwrite: boolean): Promise<InitProjectResult>
  listDir(dir: string): Promise<FileEntry[]>
  listModels(): Promise<ModelInfo[]>
  listSections(): Promise<Section[]>
  listSessions(): Promise<SessionMeta[]>
  liveSessions(): Promise<LiveSession[]>
  /** Yan bölme için: sohbeti etkin yapmadan belleğe yükler ve dökümünü döner */
  loadSessionView(id: string): Promise<TranscriptItem[]>
  /** null: bölümden çıkarır (proje grubuna döner) */
  moveSessionToSection(id: string, sectionId: string | null): Promise<void>
  openExternal(url: string): Promise<void>
  /** Kayıtlı veya bellekte çalışan sohbeti açar; arayüz bu sohbetin durumunu tutmuyorsa döküm döner */
  openSession(id: string): Promise<TranscriptItem[]>
  openSessionWindow(id: string): Promise<void>
  openWorkspace(): Promise<string | null>
  /** Çalışma dizininden dosya seçtirir, göreli yolları döner */
  pickFiles(): Promise<string[]>
  pinSession(id: string, pinned: boolean): Promise<void>
  readFile(path: string): Promise<string>
  renameSection(id: string, name: string): Promise<void>
  /** null veya boş: kullanıcı adı kaldırılır, otomatik başlığa dönülür */
  renameSession(id: string, title: string | null): Promise<void>
  revertFile(sessionId: string, path: string): Promise<void>
  /** openFile: yan panelde açık dosya (otomatik bağlam); null: açık dosya yok, verilmezse değişmez */
  send(sessionId: string, text: string, attachments: string[], openFile?: string | null): Promise<void>
  /** Bildirimlerin doğru yönlendirilmesi için arayüzde görünen sohbet; null = yeni sohbet taslağı */
  setActiveSession(id: string | null): Promise<void>
  setApiKey(key: string): Promise<void>
  setCompactSettings(patch: CompactSettingsPatch): Promise<void>
  setDefaultPermissionMode(mode: PermissionMode): Promise<void>
  setLastSession(id: string | null): Promise<void>
  setMemory(text: string): Promise<void>
  setModel(model: string): Promise<void>
  setPermissionMode(mode: PermissionMode): Promise<void>
  setSessionArchived(id: string, archived: boolean): Promise<void>
  setSessionConfig(id: string, config: { model?: string; permissionMode?: PermissionMode }): Promise<void>
  setSystemPrompt(text: string): Promise<void>
  /** Arayüz ve arka uç mesajlarının dilini kullanıcı `config.toml` dosyasına yazar. */
  setUiLanguage(lang: 'auto' | 'tr' | 'en' | 'fr'): Promise<void>
  /** Son kullanılan bir klasöre diyalog açmadan geçer. */
  setWorkspacePath(path: string): Promise<string | null>
  onEvent(listener: (event: AgentEvent) => void): () => void
  onRuntimeProgress(listener: (tick: InstallTick & { id: string }) => void): () => void
  /** Başka pencereden/arka plandan sohbet listesi veya bölümler değişince */
  onSessionsChanged(listener: () => void): () => void
  /** Ajan tarayıcısı paneli: sekmeler, adres, konsol (bkz. ipc-browser.ts) */
  abrowser: BrowserPanelApi
  /** Agent Client Protocol (§28.8, ADR 0057) */
  acp: AcpApi
  agent: AgentApi
  /** Yüklü ajanlar (bkz. ipc-agents.ts) */
  agents: AgentsApi
  analysis: AnalysisApi
  audit: AuditApi
  batch: BatchApi
  bench: BenchApi
  catalog: CatalogApi
  /** Kor tarayıcı eklentisi: eşleştirme, kurulum, alan adı listeleri, form profilleri (ADR 0058) */
  chromeExtension: ChromeExtensionApi
  /** Dosyadan slash komutları (bkz. ipc-commands.ts) */
  commands: CommandsApi
  compat: CompatApi
  components: ComponentsApi
  computer: ComputerApi
  /** [g31] SSH sunucuları ve veritabanları (bkz. ipc-connections.ts) */
  connections: ConnectionsApi
  context: ContextApi
  /** Ayarlar > Çekirdek: CLI komutlarının karşılıkları (gizlilik, undo, compat, config, pazarlar) */
  core: CoreApi
  cost: CostApi
  db: DbOpsApi
  deploy: DeployApi
  deps: DepsApi
  /** Canlı diyagramlar (§27.14, ADR 0063; ipc-ek14.ts) */
  diagram: Ek14DiagramApi
  /** Ajan için özel dokümanlar (§28.7, ADR 0063; ipc-ek14.ts) */
  docs: Ek14DocsApi
  env: EnvApi
  /** Süre ve maliyet tahmini (§27.16, ADR 0063; ipc-ek14.ts) */
  estimate: Ek14EstimateApi
  fim: FimApi
  find: FindApi
  forge: ForgeApi
  /** Konuşma dalları (§27.3, ADR 0063; ipc-ek14.ts) */
  fork: Ek14ForkApi
  /** Yarım tur kurtarma, ölü mektup kuyruğu, başlangıç öz-denetimi (bkz. ipc-g10.ts) */
  g10: G10Api
  /** Sistem istemi katmanları, .env yöneticisi, doküman kaynakları ve bağlam dışa aktarma (bkz. ipc-g13.ts) */
  g13: G13Api
  /** Gözlem panoları: izler, metrikler, maliyet, süreçler, ham istek denetçisi (bkz. ipc-g14.ts) */
  g14: G14Api
  g16: G16Api
  g30: G30Api
  /** Web hesabı oturumu: giriş, 2FA kodu, çıkış, durum (bkz. ipc-g35.ts) */
  g35: G35Api
  /** Dosya ağacı, önizleme, editör, dosya işlemleri, izleme, kontrol noktaları, tanılama (bkz. ipc-g4.ts) */
  g4: G4Api
  /** Sağlayıcılar, anahtar profilleri, uç noktalar, sağlık, maliyet, yerel Ollama (bkz. ipc-g9.ts) */
  g9: G9Api
  /** Git paneli: durum, dallar, push, PR (bkz. ipc-git.ts) */
  git: GitApi
  gitflow: GitflowApi
  graph: GraphApi
  /** Görsel hook editörü (§28.12, ADR 0063; ipc-ek14.ts) */
  hooks: Ek14HooksApi
  intent: IntentApi
  /** Kişisel bilgi tabanı ekleri (§27.15, ADR 0063; ipc-ek14.ts) */
  know: Ek14KnowApi
  korfolder: KorFolderApi
  layout: LayoutApi
  /** Günlükler paneli (bkz. ipc-logs.ts) */
  logs: LogsApi
  /** MCP sunucuları: ekle/güncelle/sil/etkinleştir/yeniden bağlan + durum (bkz. ipc-mcp.ts) */
  mcp: McpApi
  /** MCP geliştirici modu (§28.15, ADR 0063; ipc-ek14.ts) */
  mcpdev: Ek14McpDevApi
  mcpgen: McpGenApi
  media: MediaApi
  memory: MemoryApi
  metrics: MetricsApi
  /** Notlar paneli (bkz. ipc-notes.ts) */
  notes: NotesApi
  offline: OfflineApi
  osextra: OsExtraApi
  osint: OsintApi
  packages: PackagesApi
  /** Kor-to-Kor köprüsü (§31.1, ADR 0057) */
  peer: PeerApi
  perf: PerfApi
  /** Performans bindirmesi (Ctrl+Shift+P), yerel perf.log, düşük güç modu ve `kor bench --ui` masaüstü ölçümleri (§32, ADR 0062) */
  perfmon: PerfmonApi
  plan: PlanApi
  /** Eklenti paketleri (bkz. ipc-plugins.ts) */
  plugins: PluginsApi
  prefs: PrefsApi
  /** Sistem gereksinimleri, Katman C: Docker/Podman, WSL2, Xcode araçları, WebView2, macOS izinleri (§24.3, ADR 0064; bkz. ipc-prereq.ts) */
  prereq: PrereqApi
  /** Web önizleme: dev sunucusu + tarayıcı görünümü + companion seçimleri (bkz. ipc-preview.ts) */
  preview: PreviewApi
  /** Sağlayıcı gizlilik kartı (§31.2, ADR 0057) */
  privacy: PrivacyApi
  projmem: ProjMemApi
  provider: ProviderApi
  proxy: ProxyApi
  remote: RemoteApi
  replay: ReplayApi
  routines: RoutinesApi
  /** Kural uyum denetçisi (§28.3, ADR 0063; ipc-ek14.ts) */
  rules: Ek14RulesApi
  sandbox: SandboxApi
  seo: SeoApi
  sessions: SessionsApi
  shell: ShellApi
  shellui: ShellUiApi
  system: SystemApi
  /** Etiketler: @ad ile eklenen metin parçacıkları (bkz. ipc-tags.ts) */
  tags: TagsApi
  tasks: TasksApi
  team: TeamApi
  /** [cdx] Takımlar: rol tabanlı ajan ekipleri (bkz. ipc-teams.ts) */
  teams: TeamsApi
  /** Görev şablonları (bkz. ipc-templates.ts) */
  templates: TemplatesApi
  terminal: TerminalApi
  tools: ToolsApi
  triggers: TriggersApi
  vault: VaultApi
  visual: VisualApi
  watch: WatchApi
  web: WebApi
  webview: WebviewApi
  window: WindowEvents
}

/** `abrowser.*` bridge methods. */
export interface BrowserPanelApi {
  activate(id: string): Promise<void>
  back(): Promise<void>
  close(id: string): Promise<void>
  forward(): Promise<void>
  hide(): Promise<void>
  logs(): Promise<{ level: string; text: string; at: number }[]>
  navigate(url: string): Promise<void>
  newTab(url?: string): Promise<void>
  reload(): Promise<void>
  setBounds(bounds: BrowserBounds): Promise<void>
  show(bounds: BrowserBounds): Promise<void>
  tabs(): Promise<BrowserTabInfo[]>
  onEvent(listener: (event: BrowserPanelEvent) => void): () => void
}

/** `acp.*` bridge methods. */
export interface AcpApi {
  /** Editörün Kor'u ACP ajanı olarak başlatma komutu (§28.8) */
  setup(): Promise<AcpSetup>
  agents(): Promise<AcpAgentRow[]>
  /** Bir ACP ajanını alt-ajan olarak çalıştır */
  run(agent: string, prompt: string): Promise<AcpRunResult>
  answer(id: number, optionId: string | null): Promise<true>
  onUpdate(listener: (e: AcpUpdate) => void): () => void
  onPermission(listener: (e: AcpPermission) => void): () => void
}

/** `agent.*` bridge methods. */
export interface AgentApi {
  /** Same body as the `status` item of `kor:event`, with its session. */
  onStatus(listener: (payload: AgentStatusEvent) => void): () => void
}

/** `agents.*` bridge methods. */
export interface AgentsApi {
  /** Yerleşik ve gömülü ajanlar + ~/.kor/agents + <proje>/.kor/agents + etkin eklentilerin agents/. Gömülü tanımlar güven kapısından bağımsızdır. */
  list(): Promise<AgentSummary[]>
}

/** `analysis.*` bridge methods. */
export interface AnalysisApi {
  compliance(regime: 'kvkk' | 'gdpr'): Promise<{ report: AnalysisReport }>
  /** Son üretilen `tool` raporunu kaydetme penceresiyle yazar; `sign` denetim kaydına bağlar. */
  export(tool: string, format: ExportFormat__analysis, sign?: boolean): Promise<ExportResult>
  security(noExternal?: boolean): Promise<{ report: AnalysisReport }>
  /** Yol verilmezse açma penceresi */
  verifyReport(path?: string | null): Promise<VerifyResult>
}

/** `audit.*` bridge methods. */
export interface AuditApi {
  /** Tüm kaydı tek JSONL dosyasına kopyalar; yol boşsa Kor veri klasörüne. */
  export(path: string | null): Promise<{ path: string; entries: number; ok: boolean; head: AuditHead }>
  list(query: AuditQuery): Promise<AuditPage>
  /** KVKK/GDPR raporu; `since` `YYYY-AA-GG`. Markdown arayüz dilinde. */
  report(since: string | null): Promise<{ markdown: string; report: AuditReport }>
  verify(): Promise<AuditVerify>
}

/** `batch.*` bridge methods. */
export interface BatchApi {
  apply(id: string): Promise<BatchApplied>
  cancel(id: string): Promise<null>
  plan(files: string[], instruction: string, model?: string): Promise<BatchPlan>
}

/** `bench.*` bridge methods. */
export interface BenchApi {
  cancel(): Promise<{ cancelled: boolean }>
  estimate(opts: BenchOptions): Promise<BenchEstimate>
  result(): Promise<{ running: boolean; last: BenchDone | null }>
  start(opts: BenchOptions): Promise<{ started: boolean; needsConfirm?: boolean; estimate: BenchEstimate }>
  onDone(listener: (d: BenchDone) => void): () => void
  onProgress(listener: (p: BenchProgress) => void): () => void
}

/** `catalog.*` bridge methods. */
export interface CatalogApi {
  get(refresh?: boolean, query?: string): Promise<CatalogResult>
  /** Hooks from `hooks.toml` and Claude Code settings (`kor hooks list`). */
  hooks(): Promise<{ hooks: HookRow[]; warnings: string[]; events: string[] }>
}

/** `chromeExtension.*` bridge methods. */
export interface ChromeExtensionApi {
  info(): Promise<ChromeBridgeInfo>
  /** Eklenti derlemesinin klasörünü dosya yöneticisinde açar (chrome ya da firefox). */
  openFolder(flavour?: string): Promise<void>
  /** Tek kullanımlık eşleştirme kodu (kor chrome pair). */
  pair(): Promise<{ code: string; expires: number }>
  /** Eşleşmiş tüm tarayıcıları unutur (kor chrome pair --revoke). */
  revoke(): Promise<void>
  /** Alan adı listeleri (kor chrome domains). */
  domains(): Promise<ChromeDomains>
  /** Alan adına izin verir (kor chrome allow). */
  allow(domain: string): Promise<ChromeDomains>
  /** Alan adını engeller (kor chrome deny). */
  deny(domain: string): Promise<ChromeDomains>
  /** Alan adını iki listeden çıkarır (kor chrome domains --remove). */
  forget(domain: string): Promise<ChromeDomains>
  /** Yerel mesajlaşma ana bilgisayarını kaydeder (kor chrome install). */
  install(browser?: string, openFolder?: boolean): Promise<{ installed: { browser: string; manifest: string }[] }>
  /** Ana bilgisayar kayıtlarını kaldırır (kor chrome uninstall). */
  uninstall(): Promise<{ removed: string[] }>
  /** browser.backend ayarı: auto, chrome, headless. */
  setBackend(backend: string): Promise<void>
  /** Form doldurucu profilleri (kor chrome profile list). */
  profiles(): Promise<string[]>
  /** Profilin değerleri (kor chrome profile show). */
  profileGet(name: string): Promise<Record<string, string>>
  /** Profili yazar (kor chrome profile set). */
  profileSet(name: string, values: Record<string, string>): Promise<void>
  /** Profili siler (kor chrome profile remove). */
  profileRemove(name: string): Promise<boolean>
  /** Okunan dokümantasyon sayfaları (kor docs cache). */
  docsCache(): Promise<{ url: string; title: string; fetchedAt: number }[]>
  /** Dokümantasyon önbelleğini boşaltır (kor docs cache --clear). */
  docsCacheClear(): Promise<number>
}

/** `commands.*` bridge methods. */
export interface CommandsApi {
  /** Açık klasör ve kullanıcı dizinlerindeki komutları döner (proje komutları yalnızca klasör güveniliyse). */
  list(): Promise<FileCommand[]>
}

/** `compat.*` bridge methods. */
export interface CompatApi {
  conflicts(): Promise<CompatConflict[]>
  export(target: CompatExportTarget): Promise<{ dir: string; files: string[]; rules: number; warnings: string[] }>
  mcp(): Promise<{ name: string; source: CompatSource }[]>
  /** Yalnızca Kor'un yüklediği bir kaynak dosyayı dosya yöneticisinde gösterir */
  reveal(path: string): Promise<null>
}

/** `components.*` bridge methods. */
export interface ComponentsApi {
  add(name: string, from?: string): Promise<ComponentView[]>
  list(): Promise<ComponentView[]>
  installAll(): Promise<ComponentView[]>
  update(): Promise<ComponentView[]>
  verify(): Promise<{
    name: string
    state: 'ok' | 'missing' | 'outdated' | 'broken' | 'unrecorded'
    detail: string
  }[]>
  remove(name: string): Promise<ComponentView[]>
}

/** `computer.*` bridge methods. */
export interface ComputerApi {
  resetTier(app: string): Promise<ComputerStatus>
  setEnabled(on: boolean): Promise<ComputerStatus>
  setTier(app: string, tier: ComputerTier): Promise<ComputerStatus>
  status(): Promise<ComputerStatus>
  stop(): Promise<ComputerStatus>
  onActivity(listener: (a: ComputerActivity) => void): () => void
}

/** `connections.*` bridge methods. */
export interface ConnectionsApi {
  list(): Promise<ConnectionsView>
  remove(id: string): Promise<ConnectionsView>
  saveDb(db: DbConnectionInput): Promise<ConnectionsView>
  saveSsh(host: SshHostInput): Promise<ConnectionsView>
  testDb(id: string): Promise<ConnectionTestResult>
  testSsh(id: string): Promise<ConnectionTestResult>
  trustHostKey(id: string, fingerprint: string): Promise<ConnectionsView>
}

/** `context.*` bridge methods. */
export interface ContextApi {
  history(limit?: number): Promise<ContextBreakdown__cost[]>
  last(): Promise<ContextLast>
  items(sessionId: string): Promise<ContextItems>
  /** Removes one part (refused while the session runs). */
  drop(sessionId: string, index: number): Promise<ContextItems>
  /** Pinned parts survive summarization. */
  pin(sessionId: string, index: number, on: boolean): Promise<ContextItems>
  onCompactPreview(listener: (payload: CompactPreviewEvent) => void): () => void
}

/** `core.*` bridge methods. */
export interface CoreApi {
  addMarketplace(name: string, source: string): Promise<Marketplace[]>
  /** kor compat status */
  compat(): Promise<CompatStatus>
  /** kor config explain + paths */
  configExplain(): Promise<ConfigExplain>
  /** kor config set <key> <value> */
  configSet(key: string, value: string): Promise<ConfigExplain>
  /** kor plugin marketplace list / add / remove */
  marketplaces(): Promise<Marketplace[]>
  /** kor plugin doctor <id> */
  pluginDoctor(id: string): Promise<{ ok: boolean; error: string | null }>
  /** kor plugin search <query> [--index <file|url>]: eşleşen dizin girdileri, metin olarak (§12.1). */
  pluginSearch(query: string, index: string | null): Promise<string>
  /** kor plugin create <dir> [--from-skill <folder>], açık projede (§12.1). */
  pluginCreate(name: string, fromSkill: string | null): Promise<string>
  /** kor plugin revoke <id> [capability] (§12.1). */
  pluginRevoke(id: string, capability: string | null): Promise<string>
  /** kor plugin run <id> [input]: WASM eklentisinin çıktısı (§12.1). */
  pluginRun(id: string, input: string): Promise<string>
  /** `privacy.mode` (kor --privacy-mode); `offline` = `[offline] enabled` / KOR_OFFLINE (ADR 0035). */
  privacy(): Promise<{ enabled: boolean; offline?: boolean }>
  removeMarketplace(name: string): Promise<Marketplace[]>
  setPrivacy(enabled: boolean): Promise<ConfigExplain>
  /** kor undo --session <id> */
  undo(session: string): Promise<UndoReport>
  /** kor undo --list */
  undoList(): Promise<UndoJournal[]>
  /** kor update channel <stable|beta|nightly> */
  updateChannel(name: 'stable' | 'beta' | 'nightly'): Promise<ConfigExplain>
  /** `kor doctor --ipc-trace` on the desktop: true starts, false stops, omitted reports. */
  ipcTrace(on?: boolean): Promise<IpcTraceState>
}

/** `cost.*` bridge methods. */
export interface CostApi {
  export(format: 'csv' | 'json', days?: number): Promise<{ text: string }>
  exportFile(format: 'csv' | 'json', days?: number): Promise<{ path: string | null }>
  quota(): Promise<{
    report: QuotaReport
    /** Çalışan ağ geçidinin (kor proxy / CLIProxyAPI) hesapları; çalışmıyorsa null. */
    proxy?: {
      url: string
      accounts: {
        account: string
        requests: number
        failed: number
        rateLimited: number
        tokens: number
        lastUsed: number
      }[]
    } | null
    lines: string[]
  }>
  settings(): Promise<{ budget: BudgetConfig; routing: RoutingConfig }>
  summary(days?: number): Promise<{ report: CostReport__cost; lines: string[] }>
  why(last?: number): Promise<{ decisions: RoutingDecision[] }>
  consensus(prompt: string, models?: string[] | null): Promise<{
    answers: { model: string; text: string }[]
    agreed: string[]
    disputed: { models: string[]; lines: string[] }[]
    agreement: number
    failed: string[][]
  }>
  shadow(): Promise<{ main: string; shadow: string; agreement: number; samples: number }[]>
}

/** `db.*` bridge methods. */
export interface DbOpsApi {
  apply(name: string, sql: string): Promise<true>
  check(sql: string): Promise<string[]>
  indexes(): Promise<{
    tables: { connection: string; table: string; indexes: string[] }[]
    suggestions: IndexSuggestion[]
  }>
  migrateNew(description: string): Promise<{ style: MigrationStyle; prompt: string }>
  trial(name: string, sql: string): Promise<string[]>
  /** Introspects a connection (`kor db connect`) and adds its tables to the graph. */
  connect(name: string, slow?: boolean): Promise<{ snapshot: DbSnapshot; path: string; tables: number }>
  refresh(name?: string | null): Promise<{ refreshed: string[]; errors: { name: string; error: string }[]; tables: number }>
  tables(): Promise<TableInfo[]>
  show(table: string): Promise<TableInfo | null>
}

/** `deploy.*` bridge methods. */
export interface DeployApi {
  envs(): Promise<DeployEnvs>
  plan(env: string): Promise<DeployPlan>
  risk(env: string): Promise<RiskReport>
  /** `displays`: kullanıcının gördüğü plan adımları; plan değiştiyse reddedilir. */
  run(env: string, approved: number[], displays: string[], rollback: boolean): Promise<{ started: true }>
  onDone(listener: (r: DeployReport) => void): () => void
  onEvent(listener: (e: { env: string; event: DeployEvent }) => void): () => void
}

/** `deps.*` bridge methods. */
export interface DepsApi {
  /** `offline`: ağa çıkmadan son önbellek sonuçları */
  check(offline?: boolean): Promise<{ report: AnalysisReport }>
  licenses(): Promise<{ report: AnalysisReport }>
  list(): Promise<DepsList>
}

/** `diagram.*` bridge methods. */
export interface Ek14DiagramApi {
  list(): Promise<unknown[]>
  sync(force: boolean): Promise<object>
  add(file: string, kind: string, format: string, params: object): Promise<object>
}

/** `docs.*` bridge methods. */
export interface Ek14DocsApi {
  list(): Promise<unknown[]>
  add(source: string, browser: boolean): Promise<object>
  pick(): Promise<unknown>
  search(query: string): Promise<unknown[]>
  refresh(id?: string | null): Promise<unknown[]>
  remove(id: string): Promise<null>
}

/** `env.*` bridge methods. */
export interface EnvApi {
  status(): Promise<EnvStatus>
}

/** `estimate.*` bridge methods. */
export interface Ek14EstimateApi {
  task(text: string, sessionId: string | null, model: string | null): Promise<unknown>
}

/** `fim.*` bridge methods. */
export interface FimApi {
  /** Mistral Codestral ile imleç konumundaki kodu tamamlar; dosya açık projenin içinde olmalı. */
  complete(file: string, line: number, col?: number, write?: boolean): Promise<{ text: string; written: boolean }>
}

/** `find.*` bridge methods. */
export interface FindApi {
  run(question: string, limit?: number, explain?: boolean, model?: string): Promise<FindResult>
}

/** `forge.*` bridge methods. */
export interface ForgeApi {
  detect(): Promise<ForgeDetect>
  personas(): Promise<{ personas: PersonaInfo[] }>
  prCreate(input: { title: string; body: string; base: string; draft: boolean }): Promise<PrRef>
  prDescribe(base?: string | null, model?: string | null): Promise<PrDescription>
  prList(): Promise<{ prs: PrInfo[] }>
  prRespond(target: string | number, model?: string | null): Promise<RespondDraft>
  prRespondApply(target: string | number, files: { path: string; content: string }[], replies: { id: string; body: string }[]): Promise<{ commit: string | null; sent: number; errors: string[] }>
  prReview(target: string | number, persona?: string | null, model?: string | null): Promise<ReviewResult>
  prReviewPost(target: string | number, report: ReviewReport): Promise<{ inline: number; folded: number }>
  prStatus(): Promise<{ branch: string; pr: PrInfo | null }>
  review(opts: ReviewOptions): Promise<ReviewResult>
}

/** `fork.*` bridge methods. */
export interface Ek14ForkApi {
  messages(id: string): Promise<unknown[]>
  create(id: string, at: number, model: string | null, noWorktree: boolean): Promise<{
    id: string
    parent: string
    at: number
    worktree?: string | null
    branch?: string | null
    rewound?: string[]
    unrestored?: string[]
    shellAfter?: boolean
  }>
  tree(id?: string | null): Promise<unknown[]>
  compare(a: string, b: string): Promise<object>
  mergePlan(id: string): Promise<object>
  mergeApply(id: string, files: unknown[], note: boolean): Promise<object>
  remove(id: string, worktree: boolean): Promise<null>
}

/** `g10.*` bridge methods. */
export interface G10Api {
  dismiss(sessionId: string): Promise<void>
  /** Başlangıç denetimindeki eksik git'i otomatik kurar (Windows: winget). */
  installGit(): Promise<{ ok: boolean; message: string }>
  listDeadLetters(): Promise<DeadLetter[]>
  /** Uygulama açılırken: kapanış/çökme nedeniyle yarıda kalmış turlar */
  listRecoverable(): Promise<RecoverableTurn[]>
  /** Hata veren son turun kurtarılabilir adımları (yoksa null) */
  recoveryInfo(sessionId: string): Promise<RecoverableTurn | null>
  removeDeadLetter(id: string): Promise<void>
  /** Tamamlanmış adımları konuşmaya geri ekler; sohbet önce açılmış olmalı */
  restore(sessionId: string): Promise<RestoreResult>
  /** Kaydı kuyruktan çıkarır ve yeniden deneme mesajını döner */
  retryDeadLetter(id: string): Promise<{ sessionId: string; prompt: string }>
  selfCheck(): Promise<CheckItem[]>
}

/** `g13.*` bridge methods. */
export interface G13Api {
  buildExport(sessionId: string, format: ExportFormat): Promise<string>
  /** .env'den değerleri silinmiş .env.example üretir; var olanın üzerine yazmaz */
  envCreateTemplate(): Promise<string>
  envFiles(): Promise<string[]>
  /** Yalnızca adlar: değerler açık "Göster" tıklamasıyla tek tek alınır */
  envKeys(file: string): Promise<EnvKey[]>
  envRemove(file: string, key: string): Promise<void>
  envReveal(file: string, key: string): Promise<string>
  envSet(file: string, key: string, value: string): Promise<void>
  /** Şablon (.env.example) karşılaştırması: yalnızca değişken adları */
  envTemplateReport(): Promise<EnvTemplateReport>
  getDocSources(): Promise<string[]>
  getLayers(sessionId: string): Promise<PromptLayers>
  /** Kaydetme penceresi; kaydedilen yol veya iptalde null */
  saveExport(sessionId: string, format: ExportFormat): Promise<string | null>
  setDocSources(sources: string[]): Promise<void>
  /** Proje katmanı sohbetin klasörüne yazılır (getLayers ile aynı kök) */
  setProjectPrompt(sessionId: string, text: string): Promise<void>
  setSessionPrompt(sessionId: string, text: string): Promise<void>
}

/** `g14.*` bridge methods. */
export interface G14Api {
  cost(days: number): Promise<CostReport>
  getSettings(): Promise<ObservabilitySettings>
  getTags(sessionId: string): Promise<string[]>
  overview(): Promise<Overview>
  /** Sağlayıcıya kimlik bilgisi göndermeden erişilebilirlik denetimi */
  probe(): Promise<HealthInfo>
  processes(): Promise<ProcessReport>
  /** 0: değerlendirmeyi kaldırır */
  rate(traceId: string, rating: 1 | -1 | 0): Promise<void>
  raw(): Promise<RawExchange[]>
  setSettings(patch: {
    /** Tamamlanan tur izlerini OTLP/JSON olarak yerel dosyaya yaz */
    traceExport?: boolean
    /** Boş: kapalı. Yalnızca http(s)://127.0.0.1 | localhost | [::1] (ör. http://127.0.0.1:4318/v1/traces) */
    otlpEndpoint?: string
    /** Prometheus /metrics uç noktası (yalnızca 127.0.0.1) */
    prometheus?: boolean
    prometheusPort?: number
    slowToolMs?: number
    turnBudgetMs?: number
    memoryBudgetMb?: number
    /** Yavaş araç ve bütçe uyarıları bildirim olarak gösterilsin */
    alerts?: boolean
  }): Promise<ObservabilitySettings>
  setTags(sessionId: string, tags: string[]): Promise<void>
  status(sessionId: string | null): Promise<ComposerStatus>
  onAlert(listener: (alert: G14Alert) => void): () => void
}

/** `g16.*` bridge methods. */
export interface G16Api {
  onUpdateAvailable(listener: (e: { version: string | null; signed: boolean }) => void): () => void
}

/** `g30.*` bridge methods. */
export interface G30Api {
  getAutoMemory(): Promise<boolean>
  pendingList(): Promise<PendingMemory[]>
  pendingResolve(id: string, action: 'save' | 'skip', text?: string): Promise<PendingMemory[]>
  setAutoMemory(on: boolean): Promise<void>
  onPending(listener: () => void): () => void
}

/** `g35.*` bridge methods. */
export interface G35Api {
  /** E-posta + parola: 2FA yoksa giriş tamamlanır */
  login(email: string, password: string): Promise<VeniceWebLoginResult>
  /** Çıkış: sunucu oturumu kaldırılır, dosya silinir */
  logout(): Promise<VeniceWebView>
  /** Kayıtlı çerezlerle JWT tazelemeyi dene */
  restore(): Promise<VeniceWebView>
  /** Oturum durumu */
  status(): Promise<VeniceWebView>
  /** E-postaya gelen / authenticator kodu ile girişi tamamla */
  submitCode(code: string): Promise<VeniceWebLoginResult>
}

/** `g4.*` bridge methods. */
export interface G4Api {
  createDir(dir: string, name: string): Promise<string>
  /** Yeni dosyanın göreli yolunu döner */
  createFile(dir: string, name: string): Promise<string>
  detectEditors(): Promise<EditorId[]>
  listAllFiles(respectGitignore: boolean): Promise<FileListing>
  listCheckpoints(sessionId: string): Promise<CheckpointInfo[]>
  listDir(dir: string, respectGitignore: boolean): Promise<G4FileEntry[]>
  /** 'preferred': Ayarlar > Editör tipi */
  openInEditor(path: string, line: number | null, editor: EditorId | 'system' | 'preferred'): Promise<OpenedWith>
  preview(path: string): Promise<FilePreview>
  previewRewind(sessionId: string, checkpointId: string): Promise<RewindPreview>
  rename(path: string, newName: string): Promise<string>
  rewind(sessionId: string, checkpointId: string, cutConversation: boolean): Promise<RewindResult>
  runDiagnostics(): Promise<DiagnosticsReport>
  /** Kalıcı silme yok: işletim sisteminin çöp kutusuna taşır */
  trash(path: string): Promise<void>
  /** Açık klasörü izlemeye başlar; önceki izleyici kapanır */
  watch(): Promise<void>
  /** expectedMtimeMs null: diskteki sürüm denetlenmeden üzerine yazılır */
  writeFile(path: string, content: string, expectedMtimeMs: number | null): Promise<SaveResult>
  onEvent(listener: (event: G4Event) => void): () => void
}

/** `g9.*` bridge methods. */
export interface G9Api {
  addKey(provider: ProviderId, label: string, key: string, backup: boolean): Promise<void>
  clearResponseCache(): Promise<void>
  costInfo(sessionId: string | null, model: string): Promise<CostInfo>
  getProviders(): Promise<ProvidersView>
  health(): Promise<ProbeResult[]>
  ollamaCancelPull(name: string): Promise<void>
  ollamaList(): Promise<OllamaView>
  ollamaPull(name: string): Promise<void>
  ollamaUnload(name: string): Promise<void>
  removeKey(profileId: string): Promise<void>
  saveSettings(patch: G9SettingsPatch): Promise<void>
  setActiveKey(provider: ProviderId, profileId: string): Promise<void>
  setEndpoint(provider: ProviderId, endpoint: ProviderEndpoint | null): Promise<void>
  setKeyBackup(profileId: string, backup: boolean): Promise<void>
  testKey(profileId: string): Promise<ProbeResult>
  /** Ollama model download progress. */
  onPullProgress(listener: (payload: PullState) => void): () => void
}

/** `git.*` bridge methods. */
export interface GitApi {
  /** Süren rebase/merge'i iptal eder. */
  abortOperation(): Promise<string>
  branches(): Promise<GitBranches>
  /** Çakışmalar çözüldükten sonra rebase'e devam eder / merge'i commit'ler. */
  continueOperation(): Promise<string>
  createPr(input: CreatePrInput): Promise<CreatePrResult>
  /** Taban dalı geçerli dala birleştirir (git merge <base>); çakışma olursa status.conflicts dolar. */
  merge(base: string): Promise<string>
  /** Açık PR'ı tarayıcıda açar. */
  openPr(url: string): Promise<void>
  /** Geçerli dalı origin'e iter (upstream yoksa -u ile kurar). */
  push(): Promise<string>
  /** Geçerli dalı taban üzerine yeniden temellendirir (git rebase <base>). */
  rebase(base: string): Promise<string>
  status(): Promise<GitStatus>
}

/** `gitflow.*` bridge methods. */
export interface GitflowApi {
  changelog(since: string | null, write: boolean): Promise<{ section: string; since: string | null; count: number; path?: string }>
  commit(message: string): Promise<{ summary: string }>
  commitMessage(split: boolean, model?: string | null): Promise<CommitProposal>
}

/** `graph.*` bridge methods. */
export interface GraphApi {
  diff(rev: string): Promise<GraphDiff>
  explain(name: string): Promise<Explanation | null>
  export(format: ExportFormat__graph, out?: string): Promise<ExportReport>
  hotspots(limit?: number): Promise<Hotspot[]>
  impact(target: string, depth?: number): Promise<Impact>
  path(from: string, to: string): Promise<PathStep[] | null>
  query(text: string, limit?: number): Promise<QueryHit[]>
  reveal(path: string): Promise<null>
  snapshot(limit?: number, community?: number | null): Promise<Snapshot>
  status(): Promise<StatusView>
  update(): Promise<UpdateStats>
  onTouched(listener: (e: GraphTouchedEvent) => void): () => void
  onUpdated(listener: (e: GraphUpdatedEvent) => void): () => void
}

/** `hooks.*` bridge methods. */
export interface Ek14HooksApi {
  events(): Promise<object>
  files(): Promise<unknown[]>
  list(): Promise<object>
  save(fileIndex: number, item: object, replace: unknown): Promise<null>
  remove(fileIndex: number, event: string, id: string): Promise<null>
  test(fileIndex: unknown, item: object, sample: object, execute: boolean): Promise<object>
  log(limit: number): Promise<unknown[]>
}

/** `intent.*` bridge methods. */
export interface IntentApi {
  forget(text: string): Promise<{ ok: boolean }>
  learn(text: string, command: string): Promise<{ ok: boolean }>
  list(): Promise<{ intents: IntentEntry[]; learned: IntentPattern[]; errors: string[] }>
  route(text: string, ctx?: IntentRouteContext): Promise<IntentDecision>
  run(command: string, argv: string[]): Promise<IntentRunResult>
  schedule(prompt: string, intervalMinutes: number, firstRun: number): Promise<{ id: string }>
}

/** `know.*` bridge methods. */
export interface Ek14KnowApi {
  search(query: string, here: boolean): Promise<unknown[]>
  /** Bilgi tabanını okunabilir Markdown klasörüne yazar (~/Kor/Bilgi ya da [memory] folder); tek yönlü. */
  folder(): Promise<{ path: string; written: number; removed: number }>
  digest(week?: string | null): Promise<object>
  digests(): Promise<unknown[]>
  stale(here: boolean): Promise<unknown[]>
}

/** `korfolder.*` bridge methods. */
export interface KorFolderApi {
  disable(): Promise<KorFolderRemove>
  enable(): Promise<KorFolderSync>
  mount(at?: string): Promise<KorFolderMount | null>
  open(): Promise<null>
  status(): Promise<KorFolderStatus>
  sync(): Promise<KorFolderPass>
  unmount(): Promise<null>
  onChanged(listener: (p: KorFolderPass) => void): () => void
}

/** `layout.*` bridge methods. */
export interface LayoutApi {
  delete(name: string): Promise<null>
  linked(): Promise<string | null>
  list(): Promise<{ layouts: WindowLayout[]; linked: string | null }>
  restore(name: string): Promise<LayoutPlan>
  save(name: string, linkProject: boolean): Promise<WindowLayout>
  onRestored(listener: (e: { name: string; plan: LayoutPlan }) => void): () => void
}

/** `logs.*` bridge methods. */
export interface LogsApi {
  /** Sohbetin araç çağrısı günlüğü (en yeni sonda), en çok `limit` kayıt; sessionId null ise tüm sohbetler. */
  list(sessionId: string | null, limit?: number): Promise<LogEntry[]>
}

/** `mcp.*` bridge methods. */
export interface McpApi {
  /** Yapılandırmayı ve mevcut bağlantı durumunu döner (yeniden bağlanmaz). */
  list(): Promise<McpServerView[]>
  /** Tüm etkin sunuculara yeniden bağlanır. */
  reconnect(): Promise<McpServerView[]>
  remove(id: string): Promise<McpServerView[]>
  /** Sunucuyu ekler/günceller (aynı id değiştirilir), yapılandırmayı uygular ve güncel görünümü döner. */
  save(server: McpServerConfig): Promise<McpServerView[]>
  setEnabled(id: string, enabled: boolean): Promise<McpServerView[]>
}

/** `mcpdev.*` bridge methods. */
export interface Ek14McpDevApi {
  start(path: string, command: string | null): Promise<object>
  list(): Promise<unknown[]>
  inspect(id: number): Promise<object>
  reload(id: number): Promise<object>
  call(id: number, tool: string, argsJson: string): Promise<unknown>
  read(id: number, uri: string): Promise<unknown>
  prompt(id: number, name: string, argsJson: string): Promise<unknown>
  log(id: number): Promise<unknown[]>
  stop(id: number): Promise<boolean>
  onTraffic(listener: (e: object) => void): () => void
  onReloaded(listener: (e: object) => void): () => void
}

/** `mcpgen.*` bridge methods. */
export interface McpGenApi {
  add(req: McpGenRequest): Promise<McpGenAdded>
  test(id: string): Promise<McpTestReport>
}

/** `media.*` bridge methods. */
export interface MediaApi {
  cancel(jobId?: string): Promise<{ cancelled: string[] }>
  hfDownload(repo: string, file?: string, include?: string): Promise<{ jobId: string; files: { path: string; verified: boolean }[] }>
  hfInfo(repo: string): Promise<HfInfo>
  hfSearch(query?: string, pipeline?: string, filter?: string): Promise<{ models: HfModel[] }>
  jobs(): Promise<MediaJob[]>
  /** fal.ai model list, or with `model` its input JSON Schema (free-form, from fal) and price. */
  falModels(query?: string | null, category?: string | null, model?: string | null): Promise<{ models: FalModel[] } | { model: string; schema: unknown; price: MediaPrice | null }>
  falRun(model: string, input?: object): Promise<MediaRunResult>
  falImage(prompt: string, model?: string | null): Promise<MediaRunResult>
  hfInfer(model: string, task: string, input: string, token?: string | null): Promise<{ text: string }>
  /** §20.1 yetenek kataloğu (günlük önbellek; `refresh` yeniden alır). */
  catalog(capability?: string, refresh?: boolean): Promise<MediaCatalog>
  /** §20.2: sorulacak parametreler, desteklenmeyen değerler ve yapabilen modeller, maliyet ve süre. */
  plan(capability: string, model: string, prompt: string, params?: object): Promise<MediaPlan>
  /** Planlanmış üretimi çalıştırır; ilerleme `kor:media/progress`, bitiş `kor:media/done`. */
  generate(capability: string, model: string, prompt: string, params: object, reference?: string, options?: MediaGenerateOptions): Promise<MediaGenerateResult>
  /** §20.3 projenin galerisi, yeniden eskiye. */
  gallery(): Promise<MediaGalleryItem[]>
  /** Bir galeri öğesi, türediği zincir ve ondan türeyenler. */
  item(id: string): Promise<MediaGalleryDetail>
  /** Galeri öğesini projeye kopyalar (üzerine yazmaz). */
  export(id: string, dest: string): Promise<{ path: string }>
  /** §20.4 sağlayıcıda süren, toplanmamış istekler. */
  pending(): Promise<MediaPendingJob[]>
  /** Bekleyen isteği (yoksa hepsini) bekleyip galeriye kaydeder. */
  collect(jobId?: string): Promise<{ collected: { job: string; paths: string[] }[] }>
  /** Panodaki görseli galeriye referans olarak kaydeder. */
  pasteRef(): Promise<{ id: string; path: string }>
  /** Sürüklenen dosyayı galeriye referans olarak kopyalar. */
  importRef(path: string): Promise<{ id: string; path: string }>
  onCancelled(listener: (p: { jobId: string }) => void): () => void
  onHfProgress(listener: (p: HfProgress) => void): () => void
  onProgress(listener: (p: MediaProgress) => void): () => void
  onDone(listener: (p: { jobId: string; paths: string[]; items?: MediaGalleryItem[]; report?: string }) => void): () => void
}

/** `memory.*` bridge methods. */
export interface MemoryApi {
  decisions(): Promise<{ decisions: Decision[]; dir: string }>
  export(format: 'md' | 'json', dir: string): Promise<{ count: number; dir: string }>
  forget(id: number): Promise<{ deleted: boolean }>
  link(path: string): Promise<{ added: boolean; projects: string[] }>
  notesPull(): Promise<{ added: number }>
  notesPush(): Promise<{ commit: string }>
  projects(): Promise<{ projects: string[] }>
  recall(query: string, project: string | null, limit: number | null): Promise<{ hits: RecallHit[] }>
  /** Eksik vektörleri yerel modelle üretir; model kurulu değilse hata */
  reindex(): Promise<{ embedded: number; total: number }>
  /** Yerel gömme modeli durumu (ADR 0023) */
  semantic(): Promise<{ installed: boolean; component: string; embedded: number; records: number }>
  unlink(path: string): Promise<{ removed: boolean; projects: string[] }>
  workspaceAdd(pattern: string): Promise<{ added: string[] }>
  workspaceRemove(path: string): Promise<{ removed: boolean }>
  workspaces(): Promise<{ folders: string[] }>
}

/** `metrics.*` bridge methods. */
export interface MetricsApi {
  deadCode(): Promise<DeadReport>
  deadCodePrune(includeMedium?: boolean | null, checkCmd?: string | null, testCmd?: string | null): Promise<PrunePlan>
  deadCodePublish(branch: string): Promise<{ url: string; number: number; forge: string }>
  debt(thresholdHours?: number | null): Promise<Debt>
  comment(file: string, lang: string, missing?: boolean | null, onlyMissing?: boolean | null, write?: boolean | null): Promise<{ diff: string; translated: number; added: number; kept: number; written: boolean }>
  explain(file: string, level?: ExplainLevel | null, offline?: boolean | null): Promise<{ facts: Archaeology; explanation: string | null; explanationError: string | null }>
  impact(target: string | null, mode?: ImpactMode | null, depth?: number | null): Promise<ImpactReport>
  quality(): Promise<QualityReport>
  qualityHistory(): Promise<{ snapshots: QualitySnapshot[] }>
  stats(weeks?: number | null): Promise<DevStats>
  style(): Promise<{ profile: StyleProfile; saved: boolean }>
  styleCheck(staged?: boolean | null): Promise<{ report: AnalysisReport }>
}

/** `notes.*` bridge methods. */
export interface NotesApi {
  /** Sohbete bağlı serbest metin not (yoksa boş metin). */
  get(sessionId: string): Promise<string>
  set(sessionId: string, text: string): Promise<void>
}

/** `offline.*` bridge methods. */
export interface OfflineApi {
  bundle(opts: OfflineBundleOptions): Promise<{ started: boolean }>
  import(): Promise<{ started: boolean }>
  init(opts: OfflineInitOptions): Promise<{ started: boolean }>
  plan(opts: OfflineInitOptions): Promise<OfflinePlan>
  setEnabled(on: boolean): Promise<OfflineStatus>
  status(): Promise<OfflineStatus>
  onDone(listener: (d: OfflineDone) => void): () => void
  onProgress(listener: (e: OfflineEvent) => void): () => void
}

/** `osextra.*` bridge methods. */
export interface OsExtraApi {
  /** Açık oturumu Handoff'a bildirir (`kor://resume?id=…`); null bırakır. */
  handoff(url: string | null, title?: string | null): Promise<boolean>
  /** Arayüzdeki onay penceresi onaydır. */
  install(id: string, handoffUrl?: string | null): Promise<OsExtraStatus>
  list(): Promise<OsExtraList>
  /** Linux: dört portal iznini sırayla ister. */
  portals(): Promise<PortalOutcome[]>
  /** macOS: pano başka bir Apple cihazından mı geldi (öğe açıksa). */
  remoteClipboard(): Promise<boolean>
  spotlightSync(): Promise<{ indexed: number }>
  uninstall(id: string): Promise<{ status: OsExtraStatus; left: string | null }>
}

/** `osint.*` bridge methods. */
export interface OsintApi {
  /** Pencereye bırakılan tek klasör için açma isteği; değilse null. */
  drop(paths: string[]): Promise<OsintRequest | null>
  hotkeyGet(): Promise<HotkeyStatus>
  hotkeySet(value: string): Promise<HotkeyStatus>
  /** Kullanıcı kaydına yazar: yalnızca onaydan sonra. */
  menuInstall(extended: boolean): Promise<MenuStatus>
  menuStatus(): Promise<MenuStatus>
  menuUninstall(): Promise<MenuStatus>
  /** Arayüz dinlemeye başlamadan gelen istekler; sonrası `onRequest` ile gelir. */
  pending(): Promise<OsintRequest[]>
  recent(): Promise<string[]>
  onRequest(listener: (req: OsintRequest) => void): () => void
}

/** `packages.*` bridge methods. */
export interface PackagesApi {
  affected(since?: string | null): Promise<PackagesAffected>
  cost(days?: number | null): Promise<{ rows: PackageCostRow[] }>
  deps(name: string): Promise<PackageDeps>
  list(): Promise<PackagesList>
}

/** `peer.*` bridge methods. */
export interface PeerApi {
  /** Eşleşmiş makineler, yükleri, yetkileri ve en boş makine (§31.1) */
  list(): Promise<PeerList>
  /** Tek kullanımlık eşleştirme kodu ve QR modülleri */
  pairCode(): Promise<PeerPairCode>
  join(code: string, ssh?: string): Promise<{ name: string; fingerprint: string }>
  /** Görevi makinede çalıştır (auto = en boş); açık projenin kopyası gider */
  run(peer: string, text: string, dir?: string): Promise<{ peer: string; task: string }>
  attach(peer: string, task: string): Promise<true>
  approve(peer: string, task: string, id: string, approval: 'yes' | 'no' | 'always'): Promise<true>
  abort(peer: string, task: string): Promise<true>
  tasks(peer: string): Promise<PeerTaskState[]>
  result(peer: string, task: string): Promise<PeerResult>
  apply(peer: string, task: string, files?: string[]): Promise<PeerApplied[]>
  /** Hafızayı üç yollu birleştirmeyle eşitle */
  sync(peer: string): Promise<PeerSyncReport>
  /** Makinenin burada yapabilecekleri (varsayılan salt-okunur) */
  allow(peer: string, grant: PeerGrant): Promise<PeerGrant>
  route(peer: string, ssh: string | null, addrs: string[]): Promise<true>
  remove(peer: string): Promise<true>
  /** Bu masaüstü eşleşmiş makinelerden görev kabul etsin mi */
  serve(on: boolean): Promise<{ listening: string | null }>
  /** Oturumu makineye yansıt; bu makine kapanırsa orada sürer */
  standby(peer: string, sessionId: string, on: boolean): Promise<true>
  onEvent(listener: (e: PeerEventPayload) => void): () => void
  onPaired(listener: (e: { name: string }) => void): () => void
  onHandoff(listener: (e: PeerHandoff) => void): () => void
}

/** `perf.*` bridge methods. */
export interface PerfApi {
  list(): Promise<PerfSummary[]>
  load(id: string): Promise<{ report: PerfReport; flamegraph: string | null; memoryFlamegraph: string | null }>
  optimizePrompt(id: string): Promise<string | null>
  /** Dosya seçme penceresi; iptalde null */
  report(): Promise<PerfReport | null>
  run(opts: PerfRunOptions): Promise<PerfReport>
}

/** `perfmon.*` bridge methods. */
export interface PerfmonApi {
  /** `[perf] log`, düşük güç modu ve ölçüm modu (`startup`/`full`, yoksa null) */
  config(): Promise<{ log: boolean; lowPower: boolean; bench: string | null }>
  /** Süreç ağacının (WebView dahil) belleği ve CPU süresi, tokio işçi/görev sayıları */
  stats(): Promise<{
    rssMb: number
    selfRssMb: number | null
    cpuMs: number
    processes: number
    workers: number | null
    tasks: number | null
    queue: number | null
    uptimeMs: number
  }>
  /** 50 ms'yi aşan bir kare ya da IPC çağrısı; yalnızca `[perf] log` açıkken perf.log'a yazılır */
  log(entry: { kind: string; what: string; ms: number; detail?: string }): Promise<null>
  /** Pencere çizildi ve etkileşime hazır; ölçüm modunda `kor bench`'e bildirilir */
  ready(): Promise<{ ms: number }>
  /** `seconds` boyunca süreç ağacının CPU payı ve bellek */
  idle(seconds?: number): Promise<{ rssMb: number; cpuPercent: number; processes: number }>
  /** Ölçüm modu: 50k dosyalık geçici ağaçta gerçek dosya araması */
  benchSearch(query: string): Promise<{ hits: number }>
  /** Ölçüm modu: sonuçları `kor bench`'e gönderir ve uygulamayı kapatır */
  benchDone(results: Record<string, PerfmonStat>): Promise<null>
  /** Ölçüm modu: hatayı `kor bench`'e gönderir ve uygulamayı kapatır */
  benchError(message: string): Promise<null>
  /** perf.log'daki en çok zaman kaybettiren 10 neden (`kor doctor --perf` ile aynı) */
  causes(): Promise<PerfmonCause[]>
}

/** `plan.*` bridge methods. */
export interface PlanApi {
  approve(id?: string): Promise<PlanInfo>
  list(): Promise<PlanInfo[]>
  show(id: string): Promise<PlanInfo>
  onApproved(listener: (payload: PlanInfo) => void): () => void
}

/** `plugins.*` bridge methods. */
export interface PluginsApi {
  /** rea gibi katalog MCP sunucusunu mcp.json'a pasif ekler (kullanıcı MCP ayarlarından etkinleştirir). */
  addMcp(id: string): Promise<CatalogItem[]>
  /** Kurulabilir katalog (rea MCP + git eklentileri), her biri kurulu mu bilgisiyle. */
  catalog(): Promise<CatalogItem[]>
  /** Her git eklentisi için uzakta güncelleme var mı (git fetch). */
  checkUpdates(): Promise<Record<string, boolean>>
  /** Katalogdaki git eklentisini klonlar (kod çalıştırmaz). */
  install(name: string): Promise<DiscoveredPlugin[]>
  /** Mağazadan/elle: https depo adresinden kurar (git clone; kod çalıştırmaz). */
  installRepo(repoUrl: string): Promise<DiscoveredPlugin[]>
  /** Kurulu eklentiler (manifest + katkı + etkin/pasif + git ile mi). */
  list(): Promise<DiscoveredPlugin[]>
  openFolder(): Promise<void>
  setEnabled(name: string, enabled: boolean): Promise<DiscoveredPlugin[]>
  /** Global mağaza: GitHub depo araması (boş sorgu => varsayılan topic). */
  storeSearch(query: string): Promise<StoreItem[]>
  uninstall(dirName: string): Promise<DiscoveredPlugin[]>
  /** Onaylı güncelleme (git pull). */
  update(dirName: string): Promise<DiscoveredPlugin[]>
  /** Kurulum sırasında git clone yüzde ilerlemesi (ana süreçten yayınlanır). Aboneliği sonlandıran işlev döner. */
  onInstallProgress(listener: (p: { name: string; percent: number }) => void): () => void
}

/** `prefs.*` bridge methods. */
export interface PrefsApi {
  agent(): Promise<AgentPrefs>
  answer(id: string, value: QuestionAnswer | CompactAnswer | ParamCardAnswer): Promise<{ ok: boolean }>
  auto(): Promise<AutoRow[]>
  doctorFix(): Promise<DoctorFix>
  keymap(): Promise<Keymap>
  /** Overloaded in TS (x-ts): askThreshold takes an AskThreshold, the other two a boolean. */
  setAgent(name: 'askThreshold', value: AskThreshold): Promise<{ ok: boolean }>
  setAgent(name: 'compactConfirm' | 'planLock', value: boolean): Promise<{ ok: boolean }>
  setAuto(key: string, on: boolean): Promise<AutoRow[]>
  setUi<K extends keyof UiPrefs>(name: K, value: UiPrefs[K]): Promise<UiPrefs>
  ui(): Promise<UiPrefs>
}

/** `prereq.*` bridge methods. */
export interface PrereqApi {
  install(id: PrereqId, manager?: string): Promise<PrereqInstallResult>
  list(): Promise<PrereqRow[]>
  onProgress(listener: (e: PrereqProgress) => void): () => void
}

/** `preview.*` bridge methods. */
export interface PreviewApi {
  back(): Promise<void>
  devStart(): Promise<DevServerStatus>
  devStatus(): Promise<DevServerStatus>
  devStop(): Promise<DevServerStatus>
  hide(): Promise<void>
  navigate(url: string): Promise<void>
  /** Geçerli önizleme sayfasını sistem tarayıcısında açar */
  openExternal(): Promise<void>
  reload(): Promise<void>
  setBounds(bounds: PreviewBounds): Promise<void>
  setInspect(enabled: boolean): Promise<void>
  /** Görünümü pencereye ekler ve (url verilirse) yükler */
  show(url: string | null, bounds: PreviewBounds): Promise<void>
  onEvent(listener: (event: PreviewEvent) => void): () => void
}

/** `privacy.*` bridge methods. */
export interface PrivacyApi {
  /** Sağlayıcı gizlilik kartları ve etkin şartlar (§31.2) */
  cards(): Promise<PrivacyCards>
  setRequire(require: string[]): Promise<string[]>
}

/** `projmem.*` bridge methods. */
export interface ProjMemApi {
  addDecision(title: string, body: string, refs: string[]): Promise<{ decisions: Decision[] }>
  overview(): Promise<MemoryOverview__memory>
  pickFolder(): Promise<string | null>
  read(file: MemoryDocFile): Promise<{ content: string; mtimeMs: number; sections: MemorySection[] }>
  recent(project: string | null, limit: number | null): Promise<{ records: KnowledgeRecord[] }>
  session(day: string): Promise<{ content: string }>
  write(file: MemoryDocFile, content: string, expectedMtimeMs: number | null): Promise<SaveResult>
}

/** `provider.*` bridge methods. */
export interface ProviderApi {
  status(id?: string): Promise<ProviderStatusRow[]>
  test(id: string, model?: string): Promise<ProviderTestResult>
  /** User-added provider (`kor provider add`): custom endpoint, local server or subscription proxy. */
  add(id: string, url: string, anthropic?: boolean, noKey?: boolean, proxy?: boolean): Promise<{
    provider: string
    path: string
    keyEnv: string | null
    advanced: boolean
    panel?: string
    check?: ProviderCheck
  }>
  /** Removes a user-added provider manifest; false when there was none. */
  remove(id: string): Promise<boolean>
  /** Local model servers found on their default (or alternate) ports. */
  discover(): Promise<DiscoveredProvider[]>
  /** Pooled key slots of a provider; only masked tails, never the key. */
  keys(id: string): Promise<{ slot: number; masked: string }[]>
  /** Stores one more pooled key in the OS keychain. */
  addKey(id: string, key: string): Promise<{ slot: number }>
  removeKey(id: string, slot: number): Promise<boolean>
}

/** `proxy.*` bridge methods. */
export interface ProxyApi {
  config(): Promise<ProxyConfig>
  /** Anahtar yalnızca `show` ile tam döner; panel bunu hiç istemez. */
  key(rotate: boolean, show: boolean): Promise<{ masked: string | null; rotated?: boolean }>
  start(port?: number | null, host?: string | null, noAuth?: boolean): Promise<ProxyStart>
  status(): Promise<ProxyStatus>
  stop(): Promise<{ running: false; stopped: boolean }>
  /** Kayıtlı abonelik kimlik dosyalarını listele (kor-proxy oauth auth dizini). */
  oauthAccounts(): Promise<ProxyOauthAccounts>
  /** Abonelik OAuth girişi; ilerleme `kor:proxy/oauth` olayıyla (tarayıcı URL veya cihaz kodu). */
  oauthLogin(provider: string): Promise<ProxyOauthLoginResult>
  oauthRefresh(fileName: string): Promise<{ fileName: string; label: string; provider: string }>
  oauthRemove(fileName: string): Promise<{ removed: string; dir: string }>
  /** Havuzdaki bir abonelik hesabının ağırlık ve önceliğini güncelle. */
  oauthSetMeta(fileName: string, weight?: number | null, priority?: number | null): Promise<true>
  /** Çoklu abonelik hesabı havuzu: yönlendirme stratejisi ve kota yeniden deneme. */
  routing(): Promise<ProxyRouting>
  routingSet(strategy: 'round-robin' | 'weighted-round-robin' | 'fill-first', requestRetry?: number | null): Promise<ProxyRouting>
  onOauth(listener: (e: {
    kind: 'browser' | 'device'
    url: string
    userCode?: string
    verificationUri?: string
    verificationUriComplete?: string | null
  }) => void): () => void
}

/** `remote.*` bridge methods. */
export interface RemoteApi {
  abort(target: string): Promise<true>
  approve(target: string, id: string, approval: 'yes' | 'no' | 'always'): Promise<true>
  attach(target: string, task: string): Promise<true>
  connect(target: string, project?: string): Promise<RemoteConnected>
  disconnect(target: string): Promise<true>
  discover(target: string, sudoSteps: string[]): Promise<Discovery>
  hosts(): Promise<RemoteHost[]>
  report(host: string): Promise<InfraSnapshot | null>
  send(target: string, text: string, task?: string): Promise<true>
  status(): Promise<RemoteStatus[]>
  tasks(target: string): Promise<true>
  onClosed(listener: (e: { target: string }) => void): () => void
  onEvent(listener: (e: RemoteEvent) => void): () => void
}

/** `replay.*` bridge methods. */
export interface ReplayApi {
  export(id: string): Promise<{ path: string | null }>
  list(): Promise<ReplaySummary[]>
  load(id: string): Promise<ReplayRecording>
}

/** `routines.*` bridge methods. */
export interface RoutinesApi {
  add(input: RoutineInput): Promise<Trigger>
  list(): Promise<Trigger[]>
  remove(id: string): Promise<boolean>
  setEnabled(id: string, enabled: boolean): Promise<boolean>
}

/** `rules.*` bridge methods. */
export interface Ek14RulesApi {
  check(staged: boolean, judge: boolean): Promise<object>
  list(): Promise<unknown[]>
  report(days: number): Promise<unknown[]>
}

/** `sandbox.*` bridge methods. */
export interface SandboxApi {
  /** Docker imajını indirir; Kor bunu kendiliğinden asla yapmaz. */
  pull(): Promise<{ image: string; ok: boolean; output: string }>
  /** `[sandbox] network` (kullanıcı ayarı); geçersiz girdi varsa hiçbir şey yazılmaz. */
  setNetwork(entries: string[]): Promise<SandboxStatus__security>
  status(): Promise<SandboxStatus__security>
  /** Güvenli öz-denetim: geçici bir projenin dışına tek dosya yazmayı dener. */
  test(): Promise<SandboxProbe>
}

/** `seo.*` bridge methods. */
export interface SeoApi {
  agents(site: string, depth: SeoDepth, agents: string[], log: string | null, verifyDns: boolean, useModel: boolean, fix: boolean, notify: boolean): Promise<SeoTeamReport>
  agentsSchedule(site: string, every: 'daily' | 'weekly'): Promise<unknown>
  audit(target: string, depth: SeoDepth, maxPages: number | null, vitals: boolean): Promise<SeoAuditResult>
  auditSchedule(target: string, depth: SeoDepth, every: 'daily' | 'weekly'): Promise<unknown>
  bingSetup(key: string): Promise<boolean>
  bingStats(kind: 'query' | 'pages', site: string): Promise<SeoBingStat[]>
  bingStatus(): Promise<boolean>
  competitorsAdd(name: string, urls: string[], ours: string | null): Promise<{ competitor?: SeoCompetitor[] }>
  competitorsList(): Promise<{ competitor?: SeoCompetitor[] }>
  competitorsRemove(name: string): Promise<boolean>
  competitorsRun(name: string | null): Promise<SeoCompDiff[]>
  competitorsSchedule(every: 'daily' | 'weekly'): Promise<unknown>
  contentApprove(slug: string): Promise<SeoContentState>
  contentList(): Promise<SeoContentState[]>
  contentNew(keyword: string, lang: string | null, fromUrl: string | null): Promise<SeoContentState>
  contentPublish(slug: string, wpSite: string | null, publish: boolean, outPath: string | null): Promise<SeoContentState>
  contentRead(slug: string): Promise<SeoContentFiles>
  contentRun(slug: string, stage: string, serp: string[], site: string | null, model: string | null): Promise<{ state: SeoContentState; check: SeoReport | null }>
  contentSave(slug: string, file: 'brief.md' | 'outline.md' | 'draft.md', text: string): Promise<boolean>
  exportReport(report: SeoReport, format: 'html' | 'pdf' | 'markdown' | 'json' | 'sarif', path: string, title: string, brand: string | null): Promise<string>
  gaReport(property: string, days: number, metrics: string, dims: string): Promise<{ start: string; end: string; rows: Record<string, string | number>[] }>
  gscDrops(site: string, days: number, top: number, alertPct: number | null, explain: boolean): Promise<{
    before: [string, string]
    now: [string, string]
    drops: SeoDrop[]
    alert: { text: string; notes: string[] } | null
    hypotheses: string | null
  }>
  gscDropsSchedule(site: string, alertPct: number, every: 'daily' | 'weekly'): Promise<unknown>
  gscLogin(): Promise<boolean>
  gscLogout(): Promise<boolean>
  gscQuery(site: string, days: number, dims: string, limit: number): Promise<{ start: string; end: string; rows: SeoRow[] }>
  gscSetup(clientJson: string): Promise<boolean>
  gscSites(): Promise<string[]>
  gscStatus(): Promise<SeoGscStatus>
  logs(paths: string[], sitemap: string | null, verifyDns: boolean): Promise<SeoLogsResult>
  metaApply(rows: SeoMetaRow[], wpSite: string, keys: 'yoast' | 'rankmath' | 'none'): Promise<SeoApplied>
  metaCollect(url: string, allImages: boolean): Promise<SeoMetaRow[]>
  metaEstimate(rows: SeoMetaRow[], model: string | null): Promise<SeoMetaEstimate>
  metaExport(rows: SeoMetaRow[], path: string): Promise<string>
  metaGenerate(url: string | null, rows: SeoMetaRow[] | null, sample: number, all: boolean, allImages: boolean, lang: string | null, model: string | null): Promise<SeoMetaRow[]>
  metaImport(path: string): Promise<SeoMetaRow[]>
  metaUndo(file: string, wpSite: string): Promise<number>
  metaValidate(rows: SeoMetaRow[]): Promise<SeoMetaRow[]>
  opportunities(site: string, days: number, dims: string, top: number, cached: boolean): Promise<{
    formula: string
    start: string
    end: string
    calibration: { checked: number; improved: number; hit_rate: number } | null
    rows: SeoOpportunity[]
  }>
  verify(target: string): Promise<SeoVerified>
  onAgent(listener: (e: SeoAgentEvent) => void): () => void
  onOauth(listener: (e: { url: string }) => void): () => void
  onProgress(listener: (p: SeoProgress) => void): () => void
}

/** `sessions.*` bridge methods. */
export interface SessionsApi {
  archive(id: string, on: boolean): Promise<{ ok: boolean }>
  color(id: string, color: SessionColor | null): Promise<{ ok: boolean }>
  export(dir?: string): Promise<{ count: number; dir: string }>
  import(file: string): Promise<{ id: string }>
  /** Claude Code ya da Cursor sohbetlerini salt okunur içe aktarır; zaten alınmış olanlar atlanır. */
  importFrom(tool: 'claude-code' | 'cursor', all?: boolean): Promise<{ imported: number; found: number }>
  list(filter?: SessionFilter): Promise<SessionEntry[]>
  mv(id: string, folder: string | null): Promise<{ ok: boolean }>
  pin(id: string, on: boolean): Promise<{ ok: boolean }>
  rename(id: string, title: string | null): Promise<{ ok: boolean }>
  search(text: string): Promise<SessionHit[]>
  suggest(id: string, apply?: boolean): Promise<{ title: string; tags: string[] }>
  tag(id: string, add: string[], remove: string[]): Promise<string[]>
}

/** `shell.*` bridge methods. */
export interface ShellApi {
  fix(model?: string | null): Promise<{ failure: ShellFailure; explanation: string; command: string | null }>
  /** `confirm` olmadan önizleme; hiçbir şey yazılmaz. */
  install(shell: string, confirm: boolean): Promise<ShellChange>
  /** Arayüzdeki onay düğmesi onaydır. */
  runFix(command: string): Promise<{ code: number; output: string }>
  status(): Promise<ShellStatus>
  uninstall(shell: string, confirm: boolean): Promise<ShellChange>
  why(model?: string | null): Promise<{ failure: ShellFailure; explanation: string }>
  /** Runs a command in the project (no shell) and records it as the last failure when it fails. */
  capture(command: string[]): Promise<{ code: number; output: string[] }>
}

/** `shellui.*` bridge methods. */
export interface ShellUiApi {
  activity(): Promise<null>
  answerMany(ids: string[], approve: boolean): Promise<{ ok: boolean; failed: { id: string; error: string }[] }>
  approve(id: string): Promise<ApprovalSource>
  deny(id: string): Promise<ApprovalSource>
  hidePopup(): Promise<null>
  notifications(): Promise<{ items: KorNotification[]; unread: number }>
  notificationsClear(ids: string[] | null): Promise<{ items: KorNotification[]; unread: number }>
  notificationsRead(ids: string[] | null): Promise<{ items: KorNotification[]; unread: number }>
  openMain(sessionId?: string | null): Promise<null>
  /** Son kullanılan bir projeyi ana pencerede açar */
  openProject(path: string): Promise<null>
  pause(paused: boolean): Promise<boolean>
  pip(show?: boolean): Promise<null>
  queue(): Promise<{ items: QueueItem[]; groups: QueueGroup[] }>
  quickTask(root: string, prompt: string): Promise<{ sessionId: string }>
  setSetting(key: ShellSettingKey, value: boolean | number | string | string[]): Promise<ShellSettings>
  settings(): Promise<ShellSettings>
  state(): Promise<ShellState>
  status(): Promise<Overview__shellui>
  stop(sessionId: string): Promise<null>
  updateOnExit(): Promise<unknown>
  onAnswered(listener: (e: { sessionId: string; id: string }) => void): () => void
  onChanged(listener: () => void): () => void
  onNewChat(listener: () => void): () => void
  onNotification(listener: (e: { notification: KorNotification; alert: Alert }) => void): () => void
  onOpenPanel(listener: (view: string) => void): () => void
  onOpenProject(listener: (path: string) => void): () => void
  onOpenSession(listener: (sessionId: string) => void): () => void
  onOpenUpdate(listener: () => void): () => void
}

/** `system.*` bridge methods. */
export interface SystemApi {
  clean(ids: string[]): Promise<CleanOutcome[]>
  cleanSuggest(): Promise<CleanCandidate[]>
  health(deep: boolean): Promise<HealthAlert[]>
  exportAll(): Promise<{ path: string; files: number; bytes: number }>
  mute(kind: string): Promise<string[]>
  profile(): Promise<SystemProfile>
  refresh(): Promise<SystemRefresh>
  onChanged(listener: (r: SystemRefresh) => void): () => void
  onHealth(listener: (a: HealthAlert) => void): () => void
}

/** `tags.*` bridge methods. */
export interface TagsApi {
  list(): Promise<Tag[]>
  /** Tüm listeyi yazar (ekle/düzenle/sil arayüzde yapılır); geçerli listeyi döner. */
  save(tags: Tag[]): Promise<Tag[]>
}

/** `tasks.*` bridge methods. */
export interface TasksApi {
  cancel(id: string): Promise<TaskState | { cancelling: true }>
  fork(id: string): Promise<TaskState>
  list(): Promise<TaskState[]>
  /** `index` 0 tabanlı alt görev sırası */
  resolve(id: string, index: number, action: 'replan' | 'drop'): Promise<{ id: string; started: boolean }>
  resume(id: string): Promise<{ id: string; started: boolean }>
  start(opts: TaskStartOptions): Promise<{ id: string; started: boolean }>
  status(id: string): Promise<TaskState>
  onProgress(listener: (s: TaskState) => void): () => void
}

/** `team.*` bridge methods. */
export interface TeamApi {
  check(): Promise<TeamCheck>
  connect(url: string, token: string | null): Promise<TeamStatus>
  disconnect(): Promise<TeamStatus>
  status(): Promise<TeamStatus>
}

/** `teams.*` bridge methods. */
export interface TeamsApi {
  list(): Promise<TeamView[]>
  /** Kullanıcı takımlarını (~/.kor/teams.json) yazar ve güncel listeyi döner. */
  saveUser(teams: { name: string; description: string; roles: TeamRoleView[] }[]): Promise<TeamView[]>
}

/** `templates.*` bridge methods. */
export interface TemplatesApi {
  /** Varsayılanlarla, zorunlu girdileri `<ad>` olarak bırakan düzenlenebilir taslak. */
  draft(name: string): Promise<RenderedTemplate>
  /** Proje > kişisel > eklenti > yerleşik; geçersiz dosyalar `warnings` içinde. */
  list(): Promise<{ templates: TaskTemplate[]; warnings: string[] }>
  /** `.kor/templates/<ad>.md` (ya da `personal` ise `~/.kor/templates`) iskeletini oluşturur. */
  'new'(name: string, personal: boolean): Promise<{ path: string }>
  /** Girdileri CLI ile aynı kurallarla doğrular; ajanı başlatmaz, istemi döner. */
  run(name: string, inputs: Record<string, string | number | boolean>): Promise<RenderedTemplate>
  show(name: string): Promise<TaskTemplate>
}

/** `terminal.*` bridge methods. */
export interface TerminalApi {
  run(command: string, cwd?: string | null): Promise<TerminalRunResult>
}

/** `tools.*` bridge methods. */
export interface ToolsApi {
  add(def: {
    name: string
    command: string
    safety: 'read' | 'write' | 'network' | 'destructive'
    description: string
    user: boolean
  }): Promise<{ path: string }>
  describe(name: string, refresh?: boolean, model?: boolean): Promise<ToolHelp>
  list(refresh?: boolean): Promise<{
    catalog: { generated_at: number; tools: ToolEntry[] }
    typed: TypedTool[]
    errors: string[]
    trusted: boolean
  }>
  pull(): Promise<{ names: string[]; rejected: string[] }>
  search(query: string): Promise<{ hits: string[] }>
  share(): Promise<{ count: number }>
}

/** `triggers.*` bridge methods. */
export interface TriggersApi {
  answer(id: string, approve: boolean): Promise<boolean>
  /** Oturum açılışında başlat (systemd kullanıcı birimi, launchd, Görev Zamanlayıcı); kaydın yeri */
  daemonInstall(): Promise<string>
  daemonLogs(lines?: number): Promise<string>
  daemonStart(): Promise<{ running: boolean; addr: string; pid: number }>
  daemonStatus(): Promise<DaemonStatus>
  daemonStop(): Promise<boolean>
  daemonUninstall(): Promise<boolean>
  history(name: string | null, limit?: number): Promise<TriggerJob[]>
  hookInstall(repo: string | null): Promise<string>
  hookUninstall(repo: string | null): Promise<boolean>
  list(): Promise<TriggerRow[]>
  notifyGet(): Promise<NotifySettings>
  notifySet(s: {
    telegram_chat?: string
    slack?: boolean
    email_to?: string
    email_from?: string
    smtp_host?: string
    smtp_port?: number
    smtp_user?: string
    approval_timeout_mins?: number
    publicUrl?: string
  }): Promise<boolean>
  notifyTest(): Promise<NotifyTest>
  remove(name: string, origin: string): Promise<boolean>
  save(t: TriggerInput, scope: 'project' | 'user'): Promise<boolean>
  test(name: string): Promise<{ job?: string; started?: boolean }>
}

/** `vault.*` bridge methods. */
export interface VaultApi {
  add(name: string, value: string): Promise<{ names: string[]; replaced: boolean }>
  /** `confirm` yoksa yalnızca önizleme; yol boşsa proje kökündeki `.env`. */
  import(path: string | null, confirm: boolean): Promise<VaultImport>
  /** Saklanan sırların adları; değerler arayüze hiç gelmez (değeri gösteren yöntem yok, ADR 0036). */
  list(): Promise<VaultList>
  remove(name: string): Promise<{ names: string[]; removed: boolean }>
}

/** `visual.*` bridge methods. */
export interface VisualApi {
  diagram(dataUrl: string, name: string, model?: string): Promise<VisualResult>
  screenshot(dataUrl: string, name: string, target: VisualTarget, model?: string): Promise<VisualResult>
  visualize(path?: string | null, format?: VisualizeFormat): Promise<VisualizeResult>
  voice(seconds?: number, language?: string | null): Promise<{ text: string }>
  voiceStatus(): Promise<VoiceStatus>
  write(files: { path: string; content: string }[]): Promise<{ written: string[] }>
}

/** `watch.*` bridge methods. */
export interface WatchApi {
  clear(id?: string | null): Promise<number>
  fixFiles(): Promise<string[]>
  fixFinish(id: string): Promise<{ url: string; number: number; forge: string }>
  fixStart(id: string): Promise<{ incident: Incident; prompt: string }>
  incidents(): Promise<Incident[]>
  list(): Promise<Watcher[]>
  start(source: string): Promise<Watcher>
  stop(id: number): Promise<boolean>
  onEvent(listener: (e: { id: number; event: WatchEvent }) => void): () => void
}

/** `web.*` bridge methods. */
export interface WebApi {
  businessInit(): Promise<unknown>
  clientAdd(o: WebArgs): Promise<unknown>
  clientCheck(o: WebArgs): Promise<unknown>
  clientLog(o: WebArgs): Promise<unknown>
  clientReport(o: WebArgs): Promise<unknown>
  clients(): Promise<ClientItem[]>
  detect(o: WebArgs): Promise<unknown>
  hostConnect(o: WebArgs): Promise<unknown>
  hostPlan(o: WebArgs): Promise<{ plan: PlanRow[]; reversible: boolean }>
  hostRemove(o: WebArgs): Promise<boolean>
  hostTask(o: WebArgs): Promise<RunReport>
  hosts(): Promise<HostItem[]>
  inventory(o: WebArgs): Promise<unknown>
  proposal(o: WebArgs): Promise<unknown>
  proposalRevise(o: WebArgs): Promise<unknown>
  redirectsBuild(o: WebArgs): Promise<unknown>
  redirectsCheck(o: WebArgs): Promise<unknown>
  redirectsExport(o: WebArgs): Promise<unknown>
  redirectsSlugs(o: WebArgs): Promise<unknown>
  sites(): Promise<SiteItem[]>
  templates(): Promise<unknown>
  ticket(o: WebArgs): Promise<unknown>
  trCheck(o: WebArgs): Promise<unknown>
  trEfatura(o: WebArgs): Promise<unknown>
  trIrsaliye(o: WebArgs): Promise<unknown>
  trKdv(o: WebArgs): Promise<unknown>
  trKeywords(o: WebArgs): Promise<unknown>
  trKvkk(o: WebArgs): Promise<unknown>
  trMarket(): Promise<unknown>
  trPayment(o: WebArgs): Promise<unknown>
  trSlug(o: WebArgs): Promise<unknown>
  wpCleanup(o: WebArgs): Promise<unknown>
  wpConnect(o: WebArgs): Promise<unknown>
  wpDoctor(o: WebArgs): Promise<unknown>
  wpHealth(o: WebArgs): Promise<unknown>
  wpIntegrity(o: WebArgs): Promise<unknown>
  wpInventory(o: WebArgs): Promise<unknown>
  wpRemove(o: WebArgs): Promise<boolean>
  wpScaffold(o: WebArgs): Promise<unknown>
  wpStaging(o: WebArgs): Promise<unknown>
  wpUpdate(o: WebArgs): Promise<unknown>
}

/** `webview.*` bridge methods. */
export interface WebviewApi {
  /** "Çalışan görev var, kapatılsın mı?" onaylandı: pencere soru sormadan kapanır. */
  closeNow(): Promise<null>
  /** Geliştirici build'i + KOR_DEVTOOLS=1 / --devtools */
  devtools(): Promise<boolean>
  /** Örnek projeyi (~/.kor/ornek-proje) hazırlar ve yolunu döner (§23.3 "Örnek projeyle dene"). */
  sampleProject(): Promise<string>
  /** Yerel/veri URL'li görseli kaydetme penceresiyle kaydeder; iptalde null. */
  saveImage(src: string): Promise<string | null>
}

/** `window.*` bridge methods. */
export interface WindowEvents {
  /** Çalışan görev varken pencere kapatılmak istendi (`kor:window/closeRequested`). */
  onCloseRequested(listener: (payload: unknown) => void): () => void
}

/** Arguments and result of every bridge method, by `ns.method` / root name. */
export interface IpcMethods {
  'abort': { args: [sessionId: string]; result: void }
  'abrowser.activate': { args: [id: string]; result: void }
  'abrowser.back': { args: []; result: void }
  'abrowser.close': { args: [id: string]; result: void }
  'abrowser.forward': { args: []; result: void }
  'abrowser.hide': { args: []; result: void }
  'abrowser.logs': { args: []; result: { level: string; text: string; at: number }[] }
  'abrowser.navigate': { args: [url: string]; result: void }
  'abrowser.newTab': { args: [url?: string]; result: void }
  'abrowser.reload': { args: []; result: void }
  'abrowser.setBounds': { args: [bounds: BrowserBounds]; result: void }
  'abrowser.show': { args: [bounds: BrowserBounds]; result: void }
  'abrowser.tabs': { args: []; result: BrowserTabInfo[] }
  'activateSession': { args: [id: string]; result: boolean }
  'agents.list': { args: []; result: AgentSummary[] }
  'analysis.compliance': { args: [regime: 'kvkk' | 'gdpr']; result: { report: AnalysisReport } }
  'analysis.export': { args: [tool: string, format: ExportFormat__analysis, sign?: boolean]; result: ExportResult }
  'analysis.security': { args: [noExternal?: boolean]; result: { report: AnalysisReport } }
  'analysis.verifyReport': { args: [path?: string | null]; result: VerifyResult }
  'approve': { args: [id: string, approval: Approval]; result: void }
  'audit.export': { args: [path: string | null]; result: { path: string; entries: number; ok: boolean; head: AuditHead } }
  'audit.list': { args: [query: AuditQuery]; result: AuditPage }
  'audit.report': { args: [since: string | null]; result: { markdown: string; report: AuditReport } }
  'audit.verify': { args: []; result: AuditVerify }
  'batch.apply': { args: [id: string]; result: BatchApplied }
  'batch.cancel': { args: [id: string]; result: null }
  'batch.plan': { args: [files: string[], instruction: string, model?: string]; result: BatchPlan }
  'bench.cancel': { args: []; result: { cancelled: boolean } }
  'bench.estimate': { args: [opts: BenchOptions]; result: BenchEstimate }
  'bench.result': { args: []; result: { running: boolean; last: BenchDone | null } }
  'bench.start': { args: [opts: BenchOptions]; result: { started: boolean; needsConfirm?: boolean; estimate: BenchEstimate } }
  'buildProjectMap': { args: [force: boolean]; result: void }
  'cancelProjectMap': { args: []; result: void }
  'catalog.get': { args: [refresh?: boolean, query?: string]; result: CatalogResult }
  'chromeExtension.info': { args: []; result: ChromeBridgeInfo }
  'chromeExtension.openFolder': { args: [flavour?: string]; result: void }
  'clearWorkspace': { args: []; result: void }
  'commands.list': { args: []; result: FileCommand[] }
  'compactSession': { args: [sessionId: string]; result: CompactionResult }
  'compat.conflicts': { args: []; result: CompatConflict[] }
  'compat.export': { args: [target: CompatExportTarget]; result: { dir: string; files: string[]; rules: number; warnings: string[] } }
  'compat.mcp': { args: []; result: { name: string; source: CompatSource }[] }
  'compat.reveal': { args: [path: string]; result: null }
  'components.add': { args: [name: string, from?: string]; result: ComponentView[] }
  'components.list': { args: []; result: ComponentView[] }
  'components.installAll': { args: []; result: ComponentView[] }
  'components.update': { args: []; result: ComponentView[] }
  'components.verify': { args: []; result: {
      name: string
      state: 'ok' | 'missing' | 'outdated' | 'broken' | 'unrecorded'
      detail: string
    }[] }
  'components.remove': { args: [name: string]; result: ComponentView[] }
  'connections.list': { args: []; result: ConnectionsView }
  'connections.remove': { args: [id: string]; result: ConnectionsView }
  'connections.saveDb': { args: [db: DbConnectionInput]; result: ConnectionsView }
  'connections.saveSsh': { args: [host: SshHostInput]; result: ConnectionsView }
  'connections.testDb': { args: [id: string]; result: ConnectionTestResult }
  'connections.testSsh': { args: [id: string]; result: ConnectionTestResult }
  'connections.trustHostKey': { args: [id: string, fingerprint: string]; result: ConnectionsView }
  'context.history': { args: [limit?: number]; result: ContextBreakdown__cost[] }
  'context.last': { args: []; result: ContextLast }
  'core.addMarketplace': { args: [name: string, source: string]; result: Marketplace[] }
  'core.compat': { args: []; result: CompatStatus }
  'core.configExplain': { args: []; result: ConfigExplain }
  'core.configSet': { args: [key: string, value: string]; result: ConfigExplain }
  'core.marketplaces': { args: []; result: Marketplace[] }
  'core.pluginDoctor': { args: [id: string]; result: { ok: boolean; error: string | null } }
  'core.pluginSearch': { args: [query: string, index: string | null]; result: string }
  'core.pluginCreate': { args: [name: string, fromSkill: string | null]; result: string }
  'core.pluginRevoke': { args: [id: string, capability: string | null]; result: string }
  'core.pluginRun': { args: [id: string, input: string]; result: string }
  'core.privacy': { args: []; result: { enabled: boolean; offline?: boolean } }
  'core.removeMarketplace': { args: [name: string]; result: Marketplace[] }
  'core.setPrivacy': { args: [enabled: boolean]; result: ConfigExplain }
  'core.undo': { args: [session: string]; result: UndoReport }
  'core.undoList': { args: []; result: UndoJournal[] }
  'core.updateChannel': { args: [name: 'stable' | 'beta' | 'nightly']; result: ConfigExplain }
  'cost.export': { args: [format: 'csv' | 'json', days?: number]; result: { text: string } }
  'cost.exportFile': { args: [format: 'csv' | 'json', days?: number]; result: { path: string | null } }
  'cost.quota': { args: []; result: {
      report: QuotaReport
      /** Çalışan ağ geçidinin (kor proxy / CLIProxyAPI) hesapları; çalışmıyorsa null. */
      proxy?: {
        url: string
        accounts: {
          account: string
          requests: number
          failed: number
          rateLimited: number
          tokens: number
          lastUsed: number
        }[]
      } | null
      lines: string[]
    } }
  'cost.settings': { args: []; result: { budget: BudgetConfig; routing: RoutingConfig } }
  'cost.summary': { args: [days?: number]; result: { report: CostReport__cost; lines: string[] } }
  'cost.why': { args: [last?: number]; result: { decisions: RoutingDecision[] } }
  'createSection': { args: [name: string]; result: Section }
  'db.apply': { args: [name: string, sql: string]; result: true }
  'db.check': { args: [sql: string]; result: string[] }
  'db.indexes': { args: []; result: {
      tables: { connection: string; table: string; indexes: string[] }[]
      suggestions: IndexSuggestion[]
    } }
  'db.migrateNew': { args: [description: string]; result: { style: MigrationStyle; prompt: string } }
  'db.trial': { args: [name: string, sql: string]; result: string[] }
  'deleteSection': { args: [id: string]; result: void }
  'deleteSession': { args: [id: string]; result: void }
  'deploy.envs': { args: []; result: DeployEnvs }
  'deploy.plan': { args: [env: string]; result: DeployPlan }
  'deploy.risk': { args: [env: string]; result: RiskReport }
  'deploy.run': { args: [env: string, approved: number[], displays: string[], rollback: boolean]; result: { started: true } }
  'deps.check': { args: [offline?: boolean]; result: { report: AnalysisReport } }
  'deps.licenses': { args: []; result: { report: AnalysisReport } }
  'deps.list': { args: []; result: DepsList }
  'duplicateSession': { args: [id: string, spinOff: boolean]; result: string }
  'editCompactionSummary': { args: [sessionId: string, body: string]; result: boolean }
  'env.status': { args: []; result: EnvStatus }
  'exportSession': { args: [id: string]; result: string | null }
  'find.run': { args: [question: string, limit?: number, explain?: boolean, model?: string]; result: FindResult }
  'forge.detect': { args: []; result: ForgeDetect }
  'forge.personas': { args: []; result: { personas: PersonaInfo[] } }
  'forge.prCreate': { args: [input: { title: string; body: string; base: string; draft: boolean }]; result: PrRef }
  'forge.prDescribe': { args: [base?: string | null, model?: string | null]; result: PrDescription }
  'forge.prList': { args: []; result: { prs: PrInfo[] } }
  'forge.prRespond': { args: [target: string | number, model?: string | null]; result: RespondDraft }
  'forge.prRespondApply': { args: [target: string | number, files: { path: string; content: string }[], replies: { id: string; body: string }[]]; result: { commit: string | null; sent: number; errors: string[] } }
  'forge.prReview': { args: [target: string | number, persona?: string | null, model?: string | null]; result: ReviewResult }
  'forge.prReviewPost': { args: [target: string | number, report: ReviewReport]; result: { inline: number; folded: number } }
  'forge.prStatus': { args: []; result: { branch: string; pr: PrInfo | null } }
  'forge.review': { args: [opts: ReviewOptions]; result: ReviewResult }
  'g10.dismiss': { args: [sessionId: string]; result: void }
  'g10.installGit': { args: []; result: { ok: boolean; message: string } }
  'g10.listDeadLetters': { args: []; result: DeadLetter[] }
  'g10.listRecoverable': { args: []; result: RecoverableTurn[] }
  'g10.recoveryInfo': { args: [sessionId: string]; result: RecoverableTurn | null }
  'g10.removeDeadLetter': { args: [id: string]; result: void }
  'g10.restore': { args: [sessionId: string]; result: RestoreResult }
  'g10.retryDeadLetter': { args: [id: string]; result: { sessionId: string; prompt: string } }
  'g10.selfCheck': { args: []; result: CheckItem[] }
  'g11Forget': { args: [sessionId: string, id: string]; result: void }
  'g11GetOverview': { args: [sessionId: string]; result: MemoryOverview }
  'g11KeepBoth': { args: [a: string, b: string]; result: void }
  'g11SetConfidence': { args: [enabled: boolean]; result: void }
  'g12AnswerBulkDelete': { args: [id: string, approved: boolean]; result: void }
  'g12GetSandbox': { args: []; result: SandboxStatus }
  'g12GetToolScopes': { args: [sessionId: string]; result: G12ToolScope[] | null }
  'g12SetSandbox': { args: [enabled: boolean]; result: SandboxStatus }
  'g12SetToolScope': { args: [sessionId: string, tool: string, level: G12ToolLevel]; result: G12ToolScope[] }
  'g13.buildExport': { args: [sessionId: string, format: ExportFormat]; result: string }
  'g13.envCreateTemplate': { args: []; result: string }
  'g13.envFiles': { args: []; result: string[] }
  'g13.envKeys': { args: [file: string]; result: EnvKey[] }
  'g13.envRemove': { args: [file: string, key: string]; result: void }
  'g13.envReveal': { args: [file: string, key: string]; result: string }
  'g13.envSet': { args: [file: string, key: string, value: string]; result: void }
  'g13.envTemplateReport': { args: []; result: EnvTemplateReport }
  'g13.getDocSources': { args: []; result: string[] }
  'g13.getLayers': { args: [sessionId: string]; result: PromptLayers }
  'g13.saveExport': { args: [sessionId: string, format: ExportFormat]; result: string | null }
  'g13.setDocSources': { args: [sources: string[]]; result: void }
  'g13.setProjectPrompt': { args: [sessionId: string, text: string]; result: void }
  'g13.setSessionPrompt': { args: [sessionId: string, text: string]; result: void }
  'g14.cost': { args: [days: number]; result: CostReport }
  'g14.getSettings': { args: []; result: ObservabilitySettings }
  'g14.getTags': { args: [sessionId: string]; result: string[] }
  'g14.overview': { args: []; result: Overview }
  'g14.probe': { args: []; result: HealthInfo }
  'g14.processes': { args: []; result: ProcessReport }
  'g14.rate': { args: [traceId: string, rating: 1 | -1 | 0]; result: void }
  'g14.raw': { args: []; result: RawExchange[] }
  'g14.setSettings': { args: [patch: {
      /** Tamamlanan tur izlerini OTLP/JSON olarak yerel dosyaya yaz */
      traceExport?: boolean
      /** Boş: kapalı. Yalnızca http(s)://127.0.0.1 | localhost | [::1] (ör. http://127.0.0.1:4318/v1/traces) */
      otlpEndpoint?: string
      /** Prometheus /metrics uç noktası (yalnızca 127.0.0.1) */
      prometheus?: boolean
      prometheusPort?: number
      slowToolMs?: number
      turnBudgetMs?: number
      memoryBudgetMb?: number
      /** Yavaş araç ve bütçe uyarıları bildirim olarak gösterilsin */
      alerts?: boolean
    }]; result: ObservabilitySettings }
  'g14.setTags': { args: [sessionId: string, tags: string[]]; result: void }
  'g14.status': { args: [sessionId: string | null]; result: ComposerStatus }
  'g16CliInstall': { args: []; result: CliStatus }
  'g16CliRemove': { args: []; result: CliStatus }
  'g16CliStatus': { args: []; result: CliStatus }
  'g16ConfigExport': { args: []; result: string | null }
  'g16ConfigImport': { args: []; result: ConfigImportResult | null }
  'g16CrashExport': { args: []; result: string | null }
  'g16CrashList': { args: []; result: CrashInfo }
  'g16CrashOpenFolder': { args: []; result: void }
  'g16CrashSend': { args: []; result: boolean }
  'g16UpdateCheck': { args: []; result: UpdateStatus }
  'g16UpdateInstall': { args: []; result: void }
  'g16UpdateStatus': { args: []; result: UpdateStatus }
  'g17ExportPdf': { args: [html: string, suggestedName: string]; result: string | null }
  'g20ReadNotebook': { args: [path: string]; result: string }
  'g20RunPython': { args: [code: string]; result: G20PythonResult | null }
  'g20RunSql': { args: [dbPath: string, sql: string]; result: G20SqlResult | null }
  'g20StripTs': { args: [code: string]; result: string }
  'g21ReadDoc': { args: [path: string]; result: Uint8Array }
  'g22SetToolMode': { args: [model: string, mode: ToolMode]; result: void }
  'g22Support': { args: [model: string, attachments: string[]]; result: G22ModelSupport }
  'g27InstallRuntime': { args: [id: string]; result: { installed: boolean; version: string | null; opened: boolean } }
  'g27ListRuntimes': { args: []; result: RuntimeStatus[] }
  'g2EditMessage': { args: [sessionId: string, payload: G2EditPayload]; result: void }
  'g2ModelCaps': { args: []; result: Record<string, G2ModelCaps> }
  'g2PathForFile': { args: [file: File]; result: string }
  'g2QueueClear': { args: [sessionId: string]; result: string[] }
  'g2QueueFlush': { args: [sessionId: string]; result: void }
  'g2QueueRemove': { args: [sessionId: string, queueId: string]; result: boolean }
  'g2Regenerate': { args: [sessionId: string, payload: G2RegeneratePayload]; result: void }
  'g2ResolveDropped': { args: [absPaths: string[]]; result: G2DropResult }
  'g2SearchFiles': { args: [query: string]; result: G2FileHit[] }
  'g2Send': { args: [sessionId: string, payload: G2SendPayload]; result: G2SendResult }
  'g2StatFiles': { args: [paths: string[]]; result: G2FileStat[] }
  'g30.getAutoMemory': { args: []; result: boolean }
  'g30.pendingList': { args: []; result: PendingMemory[] }
  'g30.pendingResolve': { args: [id: string, action: 'save' | 'skip', text?: string]; result: PendingMemory[] }
  'g30.setAutoMemory': { args: [on: boolean]; result: void }
  'g35.login': { args: [email: string, password: string]; result: VeniceWebLoginResult }
  'g35.logout': { args: []; result: VeniceWebView }
  'g35.restore': { args: []; result: VeniceWebView }
  'g35.status': { args: []; result: VeniceWebView }
  'g35.submitCode': { args: [code: string]; result: VeniceWebLoginResult }
  'g3Answer': { args: [sessionId: string, callId: string, answer: InteractAnswer]; result: void }
  'g3GetOutputStyle': { args: []; result: OutputStyle }
  'g3LinkPreview': { args: [url: string]; result: LinkPreview | null }
  'g3RunCode': { args: [code: string, language: string]; result: SnippetResult | null }
  'g3SaveCode': { args: [content: string, language: string]; result: string | null }
  'g3SetOutputStyle': { args: [style: OutputStyle]; result: void }
  'g4.createDir': { args: [dir: string, name: string]; result: string }
  'g4.createFile': { args: [dir: string, name: string]; result: string }
  'g4.detectEditors': { args: []; result: EditorId[] }
  'g4.listAllFiles': { args: [respectGitignore: boolean]; result: FileListing }
  'g4.listCheckpoints': { args: [sessionId: string]; result: CheckpointInfo[] }
  'g4.listDir': { args: [dir: string, respectGitignore: boolean]; result: G4FileEntry[] }
  'g4.openInEditor': { args: [path: string, line: number | null, editor: EditorId | 'system' | 'preferred']; result: OpenedWith }
  'g4.preview': { args: [path: string]; result: FilePreview }
  'g4.previewRewind': { args: [sessionId: string, checkpointId: string]; result: RewindPreview }
  'g4.rename': { args: [path: string, newName: string]; result: string }
  'g4.rewind': { args: [sessionId: string, checkpointId: string, cutConversation: boolean]; result: RewindResult }
  'g4.runDiagnostics': { args: []; result: DiagnosticsReport }
  'g4.trash': { args: [path: string]; result: void }
  'g4.watch': { args: []; result: void }
  'g4.writeFile': { args: [path: string, content: string, expectedMtimeMs: number | null]; result: SaveResult }
  'g7AbortProbe': { args: [runId: string]; result: void }
  'g7AddPermissionRule': { args: [list: 'allow' | 'deny', rule: string]; result: void }
  'g7CheckOnline': { args: []; result: boolean }
  'g7ClearData': { args: [target: PrivacyClearTarget, mode: 'trash' | 'delete']; result: void }
  'g7GetCatalog': { args: [force: boolean]; result: CatalogResponse }
  'g7GetPermissionRules': { args: []; result: PermissionRulesInfo }
  'g7GetPrivacyInfo': { args: []; result: PrivacyInfo }
  'g7GetSettings': { args: []; result: G7Settings }
  'g7GetTrust': { args: []; result: TrustInfo | null }
  'g7GetUsageHistory': { args: [period: UsagePeriod]; result: UsageHistory }
  'g7OpenFolder': { args: [folder: PrivacyFolder]; result: void }
  'g7RemovePermissionRule': { args: [list: 'allow' | 'deny', rule: string]; result: void }
  'g7RunProbe': { args: [runId: string, model: string, prompt: string]; result: ProbeResult__model_probe }
  'g7SetEditor': { args: [prefs: EditorPrefs]; result: void }
  'g7SetGit': { args: [prefs: GitPrefs]; result: void }
  'g7SetLogs': { args: [logs: { keepDays: number }]; result: void }
  'g7SetModelAliases': { args: [aliases: Record<string, string>]; result: void }
  'g7SetNotifications': { args: [prefs: NotificationPrefs]; result: void }
  'g7SetProfiles': { args: [profiles: AgentProfile[]]; result: void }
  'g7SetScripts': { args: [scripts: ScriptPrefs]; result: void }
  'g7SetTaskModels': { args: [taskModels: TaskModels]; result: void }
  'g7SetTheme': { args: [theme: ThemeSetting]; result: void }
  'g7SetTrust': { args: [trusted: boolean]; result: void }
  'g9.addKey': { args: [provider: ProviderId, label: string, key: string, backup: boolean]; result: void }
  'g9.clearResponseCache': { args: []; result: void }
  'g9.costInfo': { args: [sessionId: string | null, model: string]; result: CostInfo }
  'g9.getProviders': { args: []; result: ProvidersView }
  'g9.health': { args: []; result: ProbeResult[] }
  'g9.ollamaCancelPull': { args: [name: string]; result: void }
  'g9.ollamaList': { args: []; result: OllamaView }
  'g9.ollamaPull': { args: [name: string]; result: void }
  'g9.ollamaUnload': { args: [name: string]; result: void }
  'g9.removeKey': { args: [profileId: string]; result: void }
  'g9.saveSettings': { args: [patch: G9SettingsPatch]; result: void }
  'g9.setActiveKey': { args: [provider: ProviderId, profileId: string]; result: void }
  'g9.setEndpoint': { args: [provider: ProviderId, endpoint: ProviderEndpoint | null]; result: void }
  'g9.setKeyBackup': { args: [profileId: string, backup: boolean]; result: void }
  'g9.testKey': { args: [profileId: string]; result: ProbeResult }
  'getCompactSettings': { args: []; result: CompactSettings }
  'getContextInfo': { args: [sessionId: string | null]; result: ContextView }
  'getFileChange': { args: [sessionId: string, path: string]; result: { before: string | null; after: string } | null }
  'getLastSession': { args: []; result: string | null }
  'getMemory': { args: []; result: string }
  'getModelPricing': { args: []; result: Record<string, { input: number; output: number }> }
  'getProjectMap': { args: []; result: ProjectMapInfo }
  'getSessionConfig': { args: [id: string]; result: { model: string; permissionMode: PermissionMode } }
  'getState': { args: []; result: AppState }
  'getSystemPrompt': { args: []; result: SystemPromptInfo }
  'getUsage': { args: [sessionId: string | null]; result: UsageInfo }
  'git.abortOperation': { args: []; result: string }
  'git.branches': { args: []; result: GitBranches }
  'git.continueOperation': { args: []; result: string }
  'git.createPr': { args: [input: CreatePrInput]; result: CreatePrResult }
  'git.merge': { args: [base: string]; result: string }
  'git.openPr': { args: [url: string]; result: void }
  'git.push': { args: []; result: string }
  'git.rebase': { args: [base: string]; result: string }
  'git.status': { args: []; result: GitStatus }
  'gitflow.changelog': { args: [since: string | null, write: boolean]; result: { section: string; since: string | null; count: number; path?: string } }
  'gitflow.commit': { args: [message: string]; result: { summary: string } }
  'gitflow.commitMessage': { args: [split: boolean, model?: string | null]; result: CommitProposal }
  'graph.diff': { args: [rev: string]; result: GraphDiff }
  'graph.explain': { args: [name: string]; result: Explanation | null }
  'graph.export': { args: [format: ExportFormat__graph, out?: string]; result: ExportReport }
  'graph.hotspots': { args: [limit?: number]; result: Hotspot[] }
  'graph.impact': { args: [target: string, depth?: number]; result: Impact }
  'graph.path': { args: [from: string, to: string]; result: PathStep[] | null }
  'graph.query': { args: [text: string, limit?: number]; result: QueryHit[] }
  'graph.reveal': { args: [path: string]; result: null }
  'graph.snapshot': { args: [limit?: number, community?: number | null]; result: Snapshot }
  'graph.status': { args: []; result: StatusView }
  'graph.update': { args: []; result: UpdateStats }
  'initProject': { args: [overwrite: boolean]; result: InitProjectResult }
  'intent.forget': { args: [text: string]; result: { ok: boolean } }
  'intent.learn': { args: [text: string, command: string]; result: { ok: boolean } }
  'intent.list': { args: []; result: { intents: IntentEntry[]; learned: IntentPattern[]; errors: string[] } }
  'intent.route': { args: [text: string, ctx?: IntentRouteContext]; result: IntentDecision }
  'intent.run': { args: [command: string, argv: string[]]; result: IntentRunResult }
  'intent.schedule': { args: [prompt: string, intervalMinutes: number, firstRun: number]; result: { id: string } }
  'korfolder.disable': { args: []; result: KorFolderRemove }
  'korfolder.enable': { args: []; result: KorFolderSync }
  'korfolder.mount': { args: [at?: string]; result: KorFolderMount | null }
  'korfolder.open': { args: []; result: null }
  'korfolder.status': { args: []; result: KorFolderStatus }
  'korfolder.sync': { args: []; result: KorFolderPass }
  'korfolder.unmount': { args: []; result: null }
  'listDir': { args: [dir: string]; result: FileEntry[] }
  'listModels': { args: []; result: ModelInfo[] }
  'listSections': { args: []; result: Section[] }
  'listSessions': { args: []; result: SessionMeta[] }
  'liveSessions': { args: []; result: LiveSession[] }
  'loadSessionView': { args: [id: string]; result: TranscriptItem[] }
  'logs.list': { args: [sessionId: string | null, limit?: number]; result: LogEntry[] }
  'mcp.list': { args: []; result: McpServerView[] }
  'mcp.reconnect': { args: []; result: McpServerView[] }
  'mcp.remove': { args: [id: string]; result: McpServerView[] }
  'mcp.save': { args: [server: McpServerConfig]; result: McpServerView[] }
  'mcp.setEnabled': { args: [id: string, enabled: boolean]; result: McpServerView[] }
  'mcpgen.add': { args: [req: McpGenRequest]; result: McpGenAdded }
  'mcpgen.test': { args: [id: string]; result: McpTestReport }
  'media.cancel': { args: [jobId?: string]; result: { cancelled: string[] } }
  'media.hfDownload': { args: [repo: string, file?: string, include?: string]; result: { jobId: string; files: { path: string; verified: boolean }[] } }
  'media.hfInfo': { args: [repo: string]; result: HfInfo }
  'media.hfSearch': { args: [query?: string, pipeline?: string, filter?: string]; result: { models: HfModel[] } }
  'media.jobs': { args: []; result: MediaJob[] }
  'memory.decisions': { args: []; result: { decisions: Decision[]; dir: string } }
  'memory.export': { args: [format: 'md' | 'json', dir: string]; result: { count: number; dir: string } }
  'memory.forget': { args: [id: number]; result: { deleted: boolean } }
  'memory.link': { args: [path: string]; result: { added: boolean; projects: string[] } }
  'memory.notesPull': { args: []; result: { added: number } }
  'memory.notesPush': { args: []; result: { commit: string } }
  'memory.projects': { args: []; result: { projects: string[] } }
  'memory.recall': { args: [query: string, project: string | null, limit: number | null]; result: { hits: RecallHit[] } }
  'memory.reindex': { args: []; result: { embedded: number; total: number } }
  'memory.semantic': { args: []; result: { installed: boolean; component: string; embedded: number; records: number } }
  'memory.unlink': { args: [path: string]; result: { removed: boolean; projects: string[] } }
  'memory.workspaceAdd': { args: [pattern: string]; result: { added: string[] } }
  'memory.workspaceRemove': { args: [path: string]; result: { removed: boolean } }
  'memory.workspaces': { args: []; result: { folders: string[] } }
  'metrics.deadCode': { args: []; result: DeadReport }
  'metrics.deadCodePrune': { args: [includeMedium?: boolean | null, checkCmd?: string | null, testCmd?: string | null]; result: PrunePlan }
  'metrics.deadCodePublish': { args: [branch: string]; result: { url: string; number: number; forge: string } }
  'metrics.debt': { args: [thresholdHours?: number | null]; result: Debt }
  'cost.consensus': { args: [prompt: string, models?: string[] | null]; result: {
      answers: { model: string; text: string }[]
      agreed: string[]
      disputed: { models: string[]; lines: string[] }[]
      agreement: number
      failed: string[][]
    } }
  'cost.shadow': { args: []; result: { main: string; shadow: string; agreement: number; samples: number }[] }
  'metrics.comment': { args: [file: string, lang: string, missing?: boolean | null, onlyMissing?: boolean | null, write?: boolean | null]; result: {
      diff: string
      translated: number
      added: number
      kept: number
      written: boolean
    } }
  'metrics.explain': { args: [file: string, level?: ExplainLevel | null, offline?: boolean | null]; result: { facts: Archaeology; explanation: string | null; explanationError: string | null } }
  'metrics.impact': { args: [target: string | null, mode?: ImpactMode | null, depth?: number | null]; result: ImpactReport }
  'metrics.quality': { args: []; result: QualityReport }
  'metrics.qualityHistory': { args: []; result: { snapshots: QualitySnapshot[] } }
  'metrics.stats': { args: [weeks?: number | null]; result: DevStats }
  'metrics.style': { args: []; result: { profile: StyleProfile; saved: boolean } }
  'metrics.styleCheck': { args: [staged?: boolean | null]; result: { report: AnalysisReport } }
  'moveSessionToSection': { args: [id: string, sectionId: string | null]; result: void }
  'notes.get': { args: [sessionId: string]; result: string }
  'notes.set': { args: [sessionId: string, text: string]; result: void }
  'offline.bundle': { args: [opts: OfflineBundleOptions]; result: { started: boolean } }
  'offline.import': { args: []; result: { started: boolean } }
  'offline.init': { args: [opts: OfflineInitOptions]; result: { started: boolean } }
  'offline.plan': { args: [opts: OfflineInitOptions]; result: OfflinePlan }
  'offline.setEnabled': { args: [on: boolean]; result: OfflineStatus }
  'offline.status': { args: []; result: OfflineStatus }
  'openExternal': { args: [url: string]; result: void }
  'openSession': { args: [id: string]; result: TranscriptItem[] }
  'openSessionWindow': { args: [id: string]; result: void }
  'openWorkspace': { args: []; result: string | null }
  'osextra.handoff': { args: [url: string | null, title?: string | null]; result: boolean }
  'osextra.install': { args: [id: string, handoffUrl?: string | null]; result: OsExtraStatus }
  'osextra.list': { args: []; result: OsExtraList }
  'osextra.portals': { args: []; result: PortalOutcome[] }
  'osextra.remoteClipboard': { args: []; result: boolean }
  'osextra.spotlightSync': { args: []; result: { indexed: number } }
  'osextra.uninstall': { args: [id: string]; result: { status: OsExtraStatus; left: string | null } }
  'osint.drop': { args: [paths: string[]]; result: OsintRequest | null }
  'osint.hotkeyGet': { args: []; result: HotkeyStatus }
  'osint.hotkeySet': { args: [value: string]; result: HotkeyStatus }
  'osint.menuInstall': { args: [extended: boolean]; result: MenuStatus }
  'osint.menuStatus': { args: []; result: MenuStatus }
  'osint.menuUninstall': { args: []; result: MenuStatus }
  'osint.pending': { args: []; result: OsintRequest[] }
  'osint.recent': { args: []; result: string[] }
  'packages.affected': { args: [since?: string | null]; result: PackagesAffected }
  'packages.cost': { args: [days?: number | null]; result: { rows: PackageCostRow[] } }
  'packages.deps': { args: [name: string]; result: PackageDeps }
  'packages.list': { args: []; result: PackagesList }
  'perf.list': { args: []; result: PerfSummary[] }
  'perf.load': { args: [id: string]; result: { report: PerfReport; flamegraph: string | null; memoryFlamegraph: string | null } }
  'perf.optimizePrompt': { args: [id: string]; result: string | null }
  'perf.report': { args: []; result: PerfReport | null }
  'perf.run': { args: [opts: PerfRunOptions]; result: PerfReport }
  'pickFiles': { args: []; result: string[] }
  'pinSession': { args: [id: string, pinned: boolean]; result: void }
  'plan.approve': { args: [id?: string]; result: PlanInfo }
  'plan.list': { args: []; result: PlanInfo[] }
  'plan.show': { args: [id: string]; result: PlanInfo }
  'plugins.addMcp': { args: [id: string]; result: CatalogItem[] }
  'plugins.catalog': { args: []; result: CatalogItem[] }
  'plugins.checkUpdates': { args: []; result: Record<string, boolean> }
  'plugins.install': { args: [name: string]; result: DiscoveredPlugin[] }
  'plugins.installRepo': { args: [repoUrl: string]; result: DiscoveredPlugin[] }
  'plugins.list': { args: []; result: DiscoveredPlugin[] }
  'plugins.openFolder': { args: []; result: void }
  'plugins.setEnabled': { args: [name: string, enabled: boolean]; result: DiscoveredPlugin[] }
  'plugins.storeSearch': { args: [query: string]; result: StoreItem[] }
  'plugins.uninstall': { args: [dirName: string]; result: DiscoveredPlugin[] }
  'plugins.update': { args: [dirName: string]; result: DiscoveredPlugin[] }
  'prefs.agent': { args: []; result: AgentPrefs }
  'prefs.answer': { args: [id: string, value: QuestionAnswer | CompactAnswer | ParamCardAnswer]; result: { ok: boolean } }
  'prefs.auto': { args: []; result: AutoRow[] }
  'prefs.doctorFix': { args: []; result: DoctorFix }
  'prefs.keymap': { args: []; result: Keymap }
  'prefs.setAgent': { args: [name: 'askThreshold' | 'compactConfirm' | 'planLock', value: AskThreshold | boolean]; result: { ok: boolean } }
  'prefs.setAuto': { args: [key: string, on: boolean]; result: AutoRow[] }
  'prefs.setUi': { args: [name: 'wrap' | 'wrapWidth' | 'maxWidth' | 'density' | 'fontSize' | 'lineHeight' | 'codeFont' | 'diffView' | 'toolOutput' | 'screenReader' | 'highContrast' | 'reducedMotion' | 'keys', value: boolean | number | LineWidth | Density | number | number | string | DiffViewMode | ToolOutput | boolean | boolean | boolean | KeyMode]; result: UiPrefs }
  'prefs.ui': { args: []; result: UiPrefs }
  'prereq.install': { args: [id: PrereqId, manager?: string]; result: PrereqInstallResult }
  'prereq.list': { args: []; result: PrereqRow[] }
  'preview.back': { args: []; result: void }
  'preview.devStart': { args: []; result: DevServerStatus }
  'preview.devStatus': { args: []; result: DevServerStatus }
  'preview.devStop': { args: []; result: DevServerStatus }
  'preview.hide': { args: []; result: void }
  'preview.navigate': { args: [url: string]; result: void }
  'preview.openExternal': { args: []; result: void }
  'preview.reload': { args: []; result: void }
  'preview.setBounds': { args: [bounds: PreviewBounds]; result: void }
  'preview.setInspect': { args: [enabled: boolean]; result: void }
  'preview.show': { args: [url: string | null, bounds: PreviewBounds]; result: void }
  'projmem.addDecision': { args: [title: string, body: string, refs: string[]]; result: { decisions: Decision[] } }
  'projmem.overview': { args: []; result: MemoryOverview__memory }
  'projmem.pickFolder': { args: []; result: string | null }
  'projmem.read': { args: [file: MemoryDocFile]; result: { content: string; mtimeMs: number; sections: MemorySection[] } }
  'projmem.recent': { args: [project: string | null, limit: number | null]; result: { records: KnowledgeRecord[] } }
  'projmem.session': { args: [day: string]; result: { content: string } }
  'projmem.write': { args: [file: MemoryDocFile, content: string, expectedMtimeMs: number | null]; result: SaveResult }
  'provider.status': { args: [id?: string]; result: ProviderStatusRow[] }
  'provider.test': { args: [id: string, model?: string]; result: ProviderTestResult }
  'proxy.config': { args: []; result: ProxyConfig }
  'proxy.key': { args: [rotate: boolean, show: boolean]; result: { masked: string | null; rotated?: boolean } }
  'proxy.start': { args: [port?: number | null, host?: string | null, noAuth?: boolean]; result: ProxyStart }
  'proxy.status': { args: []; result: ProxyStatus }
  'proxy.stop': { args: []; result: { running: false; stopped: boolean } }
  'proxy.oauthAccounts': { args: []; result: ProxyOauthAccounts }
  'proxy.oauthLogin': { args: [provider: string]; result: ProxyOauthLoginResult }
  'proxy.oauthRefresh': { args: [fileName: string]; result: { fileName: string; label: string; provider: string } }
  'proxy.oauthRemove': { args: [fileName: string]; result: { removed: string; dir: string } }
  'proxy.oauthSetMeta': { args: [fileName: string, weight?: number | null, priority?: number | null]; result: true }
  'proxy.routing': { args: []; result: ProxyRouting }
  'proxy.routingSet': { args: [strategy: 'round-robin' | 'weighted-round-robin' | 'fill-first', requestRetry?: number | null]; result: ProxyRouting }
  'readFile': { args: [path: string]; result: string }
  'remote.abort': { args: [target: string]; result: true }
  'remote.approve': { args: [target: string, id: string, approval: 'yes' | 'no' | 'always']; result: true }
  'remote.attach': { args: [target: string, task: string]; result: true }
  'remote.connect': { args: [target: string, project?: string]; result: RemoteConnected }
  'remote.disconnect': { args: [target: string]; result: true }
  'remote.discover': { args: [target: string, sudoSteps: string[]]; result: Discovery }
  'remote.hosts': { args: []; result: RemoteHost[] }
  'remote.report': { args: [host: string]; result: InfraSnapshot | null }
  'remote.send': { args: [target: string, text: string, task?: string]; result: true }
  'remote.status': { args: []; result: RemoteStatus[] }
  'remote.tasks': { args: [target: string]; result: true }
  'renameSection': { args: [id: string, name: string]; result: void }
  'renameSession': { args: [id: string, title: string | null]; result: void }
  'replay.export': { args: [id: string]; result: { path: string | null } }
  'replay.list': { args: []; result: ReplaySummary[] }
  'replay.load': { args: [id: string]; result: ReplayRecording }
  'revertFile': { args: [sessionId: string, path: string]; result: void }
  'routines.add': { args: [input: RoutineInput]; result: Trigger }
  'routines.list': { args: []; result: Trigger[] }
  'routines.remove': { args: [id: string]; result: boolean }
  'routines.setEnabled': { args: [id: string, enabled: boolean]; result: boolean }
  'sandbox.pull': { args: []; result: { image: string; ok: boolean; output: string } }
  'sandbox.setNetwork': { args: [entries: string[]]; result: SandboxStatus__security }
  'sandbox.status': { args: []; result: SandboxStatus__security }
  'sandbox.test': { args: []; result: SandboxProbe }
  'send': { args: [sessionId: string, text: string, attachments: string[], openFile?: string | null]; result: void }
  'seo.agents': { args: [site: string, depth: SeoDepth, agents: string[], log: string | null, verifyDns: boolean, useModel: boolean, fix: boolean, notify: boolean]; result: SeoTeamReport }
  'seo.agentsSchedule': { args: [site: string, every: 'daily' | 'weekly']; result: unknown }
  'seo.audit': { args: [target: string, depth: SeoDepth, maxPages: number | null, vitals: boolean]; result: SeoAuditResult }
  'seo.auditSchedule': { args: [target: string, depth: SeoDepth, every: 'daily' | 'weekly']; result: unknown }
  'seo.bingSetup': { args: [key: string]; result: boolean }
  'seo.bingStats': { args: [kind: 'query' | 'pages', site: string]; result: SeoBingStat[] }
  'seo.bingStatus': { args: []; result: boolean }
  'seo.competitorsAdd': { args: [name: string, urls: string[], ours: string | null]; result: { competitor?: SeoCompetitor[] } }
  'seo.competitorsList': { args: []; result: { competitor?: SeoCompetitor[] } }
  'seo.competitorsRemove': { args: [name: string]; result: boolean }
  'seo.competitorsRun': { args: [name: string | null]; result: SeoCompDiff[] }
  'seo.competitorsSchedule': { args: [every: 'daily' | 'weekly']; result: unknown }
  'seo.contentApprove': { args: [slug: string]; result: SeoContentState }
  'seo.contentList': { args: []; result: SeoContentState[] }
  'seo.contentNew': { args: [keyword: string, lang: string | null, fromUrl: string | null]; result: SeoContentState }
  'seo.contentPublish': { args: [slug: string, wpSite: string | null, publish: boolean, outPath: string | null]; result: SeoContentState }
  'seo.contentRead': { args: [slug: string]; result: SeoContentFiles }
  'seo.contentRun': { args: [slug: string, stage: string, serp: string[], site: string | null, model: string | null]; result: { state: SeoContentState; check: SeoReport | null } }
  'seo.contentSave': { args: [slug: string, file: 'brief.md' | 'outline.md' | 'draft.md', text: string]; result: boolean }
  'seo.exportReport': { args: [report: SeoReport, format: 'html' | 'pdf' | 'markdown' | 'json' | 'sarif', path: string, title: string, brand: string | null]; result: string }
  'seo.gaReport': { args: [property: string, days: number, metrics: string, dims: string]; result: { start: string; end: string; rows: Record<string, string | number>[] } }
  'seo.gscDrops': { args: [site: string, days: number, top: number, alertPct: number | null, explain: boolean]; result: {
      before: [string, string]
      now: [string, string]
      drops: SeoDrop[]
      alert: { text: string; notes: string[] } | null
      hypotheses: string | null
    } }
  'seo.gscDropsSchedule': { args: [site: string, alertPct: number, every: 'daily' | 'weekly']; result: unknown }
  'seo.gscLogin': { args: []; result: boolean }
  'seo.gscLogout': { args: []; result: boolean }
  'seo.gscQuery': { args: [site: string, days: number, dims: string, limit: number]; result: { start: string; end: string; rows: SeoRow[] } }
  'seo.gscSetup': { args: [clientJson: string]; result: boolean }
  'seo.gscSites': { args: []; result: string[] }
  'seo.gscStatus': { args: []; result: SeoGscStatus }
  'seo.logs': { args: [paths: string[], sitemap: string | null, verifyDns: boolean]; result: SeoLogsResult }
  'seo.metaApply': { args: [rows: SeoMetaRow[], wpSite: string, keys: 'yoast' | 'rankmath' | 'none']; result: SeoApplied }
  'seo.metaCollect': { args: [url: string, allImages: boolean]; result: SeoMetaRow[] }
  'seo.metaEstimate': { args: [rows: SeoMetaRow[], model: string | null]; result: SeoMetaEstimate }
  'seo.metaExport': { args: [rows: SeoMetaRow[], path: string]; result: string }
  'seo.metaGenerate': { args: [url: string | null, rows: SeoMetaRow[] | null, sample: number, all: boolean, allImages: boolean, lang: string | null, model: string | null]; result: SeoMetaRow[] }
  'seo.metaImport': { args: [path: string]; result: SeoMetaRow[] }
  'seo.metaUndo': { args: [file: string, wpSite: string]; result: number }
  'seo.metaValidate': { args: [rows: SeoMetaRow[]]; result: SeoMetaRow[] }
  'seo.opportunities': { args: [site: string, days: number, dims: string, top: number, cached: boolean]; result: {
      formula: string
      start: string
      end: string
      calibration: { checked: number; improved: number; hit_rate: number } | null
      rows: SeoOpportunity[]
    } }
  'seo.verify': { args: [target: string]; result: SeoVerified }
  'sessions.archive': { args: [id: string, on: boolean]; result: { ok: boolean } }
  'sessions.color': { args: [id: string, color: SessionColor | null]; result: { ok: boolean } }
  'sessions.export': { args: [dir?: string]; result: { count: number; dir: string } }
  'sessions.import': { args: [file: string]; result: { id: string } }
  'sessions.importFrom': { args: [tool: 'claude-code' | 'cursor', all?: boolean]; result: { imported: number; found: number } }
  'sessions.list': { args: [filter?: SessionFilter]; result: SessionEntry[] }
  'sessions.mv': { args: [id: string, folder: string | null]; result: { ok: boolean } }
  'sessions.pin': { args: [id: string, on: boolean]; result: { ok: boolean } }
  'sessions.rename': { args: [id: string, title: string | null]; result: { ok: boolean } }
  'sessions.search': { args: [text: string]; result: SessionHit[] }
  'sessions.suggest': { args: [id: string, apply?: boolean]; result: { title: string; tags: string[] } }
  'sessions.tag': { args: [id: string, add: string[], remove: string[]]; result: string[] }
  'setActiveSession': { args: [id: string | null]; result: void }
  'setApiKey': { args: [key: string]; result: void }
  'setCompactSettings': { args: [patch: CompactSettingsPatch]; result: void }
  'setDefaultPermissionMode': { args: [mode: PermissionMode]; result: void }
  'setLastSession': { args: [id: string | null]; result: void }
  'setMemory': { args: [text: string]; result: void }
  'setModel': { args: [model: string]; result: void }
  'setPermissionMode': { args: [mode: PermissionMode]; result: void }
  'setSessionArchived': { args: [id: string, archived: boolean]; result: void }
  'setSessionConfig': { args: [id: string, config: { model?: string; permissionMode?: PermissionMode }]; result: void }
  'setSystemPrompt': { args: [text: string]; result: void }
  'setUiLanguage': { args: [lang: 'auto' | 'tr' | 'en' | 'fr']; result: void }
  'setWorkspacePath': { args: [path: string]; result: string | null }
  'shell.fix': { args: [model?: string | null]; result: { failure: ShellFailure; explanation: string; command: string | null } }
  'shell.install': { args: [shell: string, confirm: boolean]; result: ShellChange }
  'shell.runFix': { args: [command: string]; result: { code: number; output: string } }
  'shell.status': { args: []; result: ShellStatus }
  'shell.uninstall': { args: [shell: string, confirm: boolean]; result: ShellChange }
  'shell.why': { args: [model?: string | null]; result: { failure: ShellFailure; explanation: string } }
  'shellui.activity': { args: []; result: null }
  'shellui.answerMany': { args: [ids: string[], approve: boolean]; result: { ok: boolean; failed: { id: string; error: string }[] } }
  'shellui.approve': { args: [id: string]; result: ApprovalSource }
  'shellui.deny': { args: [id: string]; result: ApprovalSource }
  'shellui.hidePopup': { args: []; result: null }
  'shellui.notifications': { args: []; result: { items: KorNotification[]; unread: number } }
  'shellui.notificationsClear': { args: [ids: string[] | null]; result: { items: KorNotification[]; unread: number } }
  'shellui.notificationsRead': { args: [ids: string[] | null]; result: { items: KorNotification[]; unread: number } }
  'shellui.openMain': { args: [sessionId?: string | null]; result: null }
  'shellui.openProject': { args: [path: string]; result: null }
  'shellui.pause': { args: [paused: boolean]; result: boolean }
  'shellui.pip': { args: [show?: boolean]; result: null }
  'shellui.queue': { args: []; result: { items: QueueItem[]; groups: QueueGroup[] } }
  'shellui.quickTask': { args: [root: string, prompt: string]; result: { sessionId: string } }
  'shellui.setSetting': { args: [key: ShellSettingKey, value: boolean | number | string | string[]]; result: ShellSettings }
  'shellui.settings': { args: []; result: ShellSettings }
  'shellui.state': { args: []; result: ShellState }
  'shellui.status': { args: []; result: Overview__shellui }
  'shellui.stop': { args: [sessionId: string]; result: null }
  'shellui.updateOnExit': { args: []; result: unknown }
  'tags.list': { args: []; result: Tag[] }
  'tags.save': { args: [tags: Tag[]]; result: Tag[] }
  'tasks.cancel': { args: [id: string]; result: TaskState | { cancelling: true } }
  'tasks.fork': { args: [id: string]; result: TaskState }
  'tasks.list': { args: []; result: TaskState[] }
  'tasks.resolve': { args: [id: string, index: number, action: 'replan' | 'drop']; result: { id: string; started: boolean } }
  'tasks.resume': { args: [id: string]; result: { id: string; started: boolean } }
  'tasks.start': { args: [opts: TaskStartOptions]; result: { id: string; started: boolean } }
  'tasks.status': { args: [id: string]; result: TaskState }
  'team.check': { args: []; result: TeamCheck }
  'team.connect': { args: [url: string, token: string | null]; result: TeamStatus }
  'team.disconnect': { args: []; result: TeamStatus }
  'team.status': { args: []; result: TeamStatus }
  'teams.list': { args: []; result: TeamView[] }
  'teams.saveUser': { args: [teams: { name: string; description: string; roles: TeamRoleView[] }[]]; result: TeamView[] }
  'templates.draft': { args: [name: string]; result: RenderedTemplate }
  'templates.list': { args: []; result: { templates: TaskTemplate[]; warnings: string[] } }
  'templates.new': { args: [name: string, personal: boolean]; result: { path: string } }
  'templates.run': { args: [name: string, inputs: Record<string, string | number | boolean>]; result: RenderedTemplate }
  'templates.show': { args: [name: string]; result: TaskTemplate }
  'terminal.run': { args: [command: string, cwd?: string | null]; result: TerminalRunResult }
  'tools.add': { args: [def: {
      name: string
      command: string
      safety: 'read' | 'write' | 'network' | 'destructive'
      description: string
      user: boolean
    }]; result: { path: string } }
  'tools.describe': { args: [name: string, refresh?: boolean, model?: boolean]; result: ToolHelp }
  'tools.list': { args: [refresh?: boolean]; result: {
      catalog: { generated_at: number; tools: ToolEntry[] }
      typed: TypedTool[]
      errors: string[]
      trusted: boolean
    } }
  'tools.pull': { args: []; result: { names: string[]; rejected: string[] } }
  'tools.search': { args: [query: string]; result: { hits: string[] } }
  'tools.share': { args: []; result: { count: number } }
  'triggers.answer': { args: [id: string, approve: boolean]; result: boolean }
  'triggers.daemonInstall': { args: []; result: string }
  'triggers.daemonLogs': { args: [lines?: number]; result: string }
  'triggers.daemonStart': { args: []; result: { running: boolean; addr: string; pid: number } }
  'triggers.daemonStatus': { args: []; result: DaemonStatus }
  'triggers.daemonStop': { args: []; result: boolean }
  'triggers.daemonUninstall': { args: []; result: boolean }
  'triggers.history': { args: [name: string | null, limit?: number]; result: TriggerJob[] }
  'triggers.hookInstall': { args: [repo: string | null]; result: string }
  'triggers.hookUninstall': { args: [repo: string | null]; result: boolean }
  'triggers.list': { args: []; result: TriggerRow[] }
  'triggers.notifyGet': { args: []; result: NotifySettings }
  'triggers.notifySet': { args: [s: {
      telegram_chat?: string
      slack?: boolean
      email_to?: string
      email_from?: string
      smtp_host?: string
      smtp_port?: number
      smtp_user?: string
      approval_timeout_mins?: number
      publicUrl?: string
    }]; result: boolean }
  'triggers.notifyTest': { args: []; result: NotifyTest }
  'triggers.remove': { args: [name: string, origin: string]; result: boolean }
  'triggers.save': { args: [t: TriggerInput, scope: 'project' | 'user']; result: boolean }
  'triggers.test': { args: [name: string]; result: { job?: string; started?: boolean } }
  'vault.add': { args: [name: string, value: string]; result: { names: string[]; replaced: boolean } }
  'vault.import': { args: [path: string | null, confirm: boolean]; result: VaultImport }
  'vault.list': { args: []; result: VaultList }
  'vault.remove': { args: [name: string]; result: { names: string[]; removed: boolean } }
  'visual.diagram': { args: [dataUrl: string, name: string, model?: string]; result: VisualResult }
  'visual.screenshot': { args: [dataUrl: string, name: string, target: VisualTarget, model?: string]; result: VisualResult }
  'visual.visualize': { args: [path?: string | null, format?: VisualizeFormat]; result: VisualizeResult }
  'visual.voice': { args: [seconds?: number, language?: string | null]; result: { text: string } }
  'visual.voiceStatus': { args: []; result: VoiceStatus }
  'visual.write': { args: [files: { path: string; content: string }[]]; result: { written: string[] } }
  'watch.clear': { args: [id?: string | null]; result: number }
  'watch.fixFiles': { args: []; result: string[] }
  'watch.fixFinish': { args: [id: string]; result: { url: string; number: number; forge: string } }
  'watch.fixStart': { args: [id: string]; result: { incident: Incident; prompt: string } }
  'watch.incidents': { args: []; result: Incident[] }
  'watch.list': { args: []; result: Watcher[] }
  'watch.start': { args: [source: string]; result: Watcher }
  'watch.stop': { args: [id: number]; result: boolean }
  'web.businessInit': { args: []; result: unknown }
  'web.clientAdd': { args: [o: WebArgs]; result: unknown }
  'web.clientCheck': { args: [o: WebArgs]; result: unknown }
  'web.clientLog': { args: [o: WebArgs]; result: unknown }
  'web.clientReport': { args: [o: WebArgs]; result: unknown }
  'web.clients': { args: []; result: ClientItem[] }
  'web.detect': { args: [o: WebArgs]; result: unknown }
  'web.hostConnect': { args: [o: WebArgs]; result: unknown }
  'web.hostPlan': { args: [o: WebArgs]; result: { plan: PlanRow[]; reversible: boolean } }
  'web.hostRemove': { args: [o: WebArgs]; result: boolean }
  'web.hostTask': { args: [o: WebArgs]; result: RunReport }
  'web.hosts': { args: []; result: HostItem[] }
  'web.inventory': { args: [o: WebArgs]; result: unknown }
  'web.proposal': { args: [o: WebArgs]; result: unknown }
  'web.proposalRevise': { args: [o: WebArgs]; result: unknown }
  'web.redirectsBuild': { args: [o: WebArgs]; result: unknown }
  'web.redirectsCheck': { args: [o: WebArgs]; result: unknown }
  'web.redirectsExport': { args: [o: WebArgs]; result: unknown }
  'web.redirectsSlugs': { args: [o: WebArgs]; result: unknown }
  'web.sites': { args: []; result: SiteItem[] }
  'web.templates': { args: []; result: unknown }
  'web.ticket': { args: [o: WebArgs]; result: unknown }
  'web.trCheck': { args: [o: WebArgs]; result: unknown }
  'web.trEfatura': { args: [o: WebArgs]; result: unknown }
  'web.trIrsaliye': { args: [o: WebArgs]; result: unknown }
  'web.trKdv': { args: [o: WebArgs]; result: unknown }
  'web.trKeywords': { args: [o: WebArgs]; result: unknown }
  'web.trKvkk': { args: [o: WebArgs]; result: unknown }
  'web.trMarket': { args: []; result: unknown }
  'web.trPayment': { args: [o: WebArgs]; result: unknown }
  'web.trSlug': { args: [o: WebArgs]; result: unknown }
  'web.wpCleanup': { args: [o: WebArgs]; result: unknown }
  'web.wpConnect': { args: [o: WebArgs]; result: unknown }
  'web.wpDoctor': { args: [o: WebArgs]; result: unknown }
  'web.wpHealth': { args: [o: WebArgs]; result: unknown }
  'web.wpIntegrity': { args: [o: WebArgs]; result: unknown }
  'web.wpInventory': { args: [o: WebArgs]; result: unknown }
  'web.wpRemove': { args: [o: WebArgs]; result: boolean }
  'web.wpScaffold': { args: [o: WebArgs]; result: unknown }
  'web.wpStaging': { args: [o: WebArgs]; result: unknown }
  'web.wpUpdate': { args: [o: WebArgs]; result: unknown }
  'webview.closeNow': { args: []; result: null }
  'webview.devtools': { args: []; result: boolean }
  'webview.sampleProject': { args: []; result: string }
  'webview.saveImage': { args: [src: string]; result: string | null }
  'catalog.hooks': { args: []; result: { hooks: HookRow[]; warnings: string[]; events: string[] } }
  'provider.add': { args: [id: string, url: string, anthropic?: boolean, noKey?: boolean, proxy?: boolean]; result: {
      provider: string
      path: string
      keyEnv: string | null
      advanced: boolean
      panel?: string
      check?: ProviderCheck
    } }
  'provider.remove': { args: [id: string]; result: boolean }
  'provider.discover': { args: []; result: DiscoveredProvider[] }
  'provider.keys': { args: [id: string]; result: { slot: number; masked: string }[] }
  'provider.addKey': { args: [id: string, key: string]; result: { slot: number } }
  'provider.removeKey': { args: [id: string, slot: number]; result: boolean }
  'media.falModels': { args: [query?: string | null, category?: string | null, model?: string | null]; result: { models: FalModel[] } | { model: string; schema: unknown; price: MediaPrice | null } }
  'media.falRun': { args: [model: string, input?: object]; result: MediaRunResult }
  'media.falImage': { args: [prompt: string, model?: string | null]; result: MediaRunResult }
  'media.hfInfer': { args: [model: string, task: string, input: string, token?: string | null]; result: { text: string } }
  'shell.capture': { args: [command: string[]]; result: { code: number; output: string[] } }
  'db.connect': { args: [name: string, slow?: boolean]; result: { snapshot: DbSnapshot; path: string; tables: number } }
  'db.refresh': { args: [name?: string | null]; result: { refreshed: string[]; errors: { name: string; error: string }[]; tables: number } }
  'db.tables': { args: []; result: TableInfo[] }
  'db.show': { args: [table: string]; result: TableInfo | null }
  'context.items': { args: [sessionId: string]; result: ContextItems }
  'context.drop': { args: [sessionId: string, index: number]; result: ContextItems }
  'context.pin': { args: [sessionId: string, index: number, on: boolean]; result: ContextItems }
  'core.ipcTrace': { args: [on?: boolean]; result: IpcTraceState }
  'perfmon.config': { args: []; result: { log: boolean; lowPower: boolean; bench: string | null } }
  'perfmon.stats': { args: []; result: {
      rssMb: number
      selfRssMb: number | null
      cpuMs: number
      processes: number
      workers: number | null
      tasks: number | null
      queue: number | null
      uptimeMs: number
    } }
  'perfmon.log': { args: [entry: { kind: string; what: string; ms: number; detail?: string }]; result: null }
  'perfmon.ready': { args: []; result: { ms: number } }
  'perfmon.idle': { args: [seconds?: number]; result: { rssMb: number; cpuPercent: number; processes: number } }
  'perfmon.benchSearch': { args: [query: string]; result: { hits: number } }
  'perfmon.benchDone': { args: [results: Record<string, PerfmonStat>]; result: null }
  'perfmon.benchError': { args: [message: string]; result: null }
  'perfmon.causes': { args: []; result: PerfmonCause[] }
  'media.catalog': { args: [capability?: string, refresh?: boolean]; result: MediaCatalog }
  'media.plan': { args: [capability: string, model: string, prompt: string, params?: object]; result: MediaPlan }
  'media.generate': { args: [capability: string, model: string, prompt: string, params: object, reference?: string, options?: MediaGenerateOptions]; result: MediaGenerateResult }
  'media.gallery': { args: []; result: MediaGalleryItem[] }
  'media.item': { args: [id: string]; result: MediaGalleryDetail }
  'media.export': { args: [id: string, dest: string]; result: { path: string } }
  'media.pending': { args: []; result: MediaPendingJob[] }
  'media.collect': { args: [jobId?: string]; result: { collected: { job: string; paths: string[] }[] } }
  'media.pasteRef': { args: []; result: { id: string; path: string } }
  'media.importRef': { args: [path: string]; result: { id: string; path: string } }
  'peer.list': { args: []; result: PeerList }
  'peer.pairCode': { args: []; result: PeerPairCode }
  'peer.join': { args: [code: string, ssh?: string]; result: { name: string; fingerprint: string } }
  'peer.run': { args: [peer: string, text: string, dir?: string]; result: { peer: string; task: string } }
  'peer.attach': { args: [peer: string, task: string]; result: true }
  'peer.approve': { args: [peer: string, task: string, id: string, approval: 'yes' | 'no' | 'always']; result: true }
  'peer.abort': { args: [peer: string, task: string]; result: true }
  'peer.tasks': { args: [peer: string]; result: PeerTaskState[] }
  'peer.result': { args: [peer: string, task: string]; result: PeerResult }
  'peer.apply': { args: [peer: string, task: string, files?: string[]]; result: PeerApplied[] }
  'peer.sync': { args: [peer: string]; result: PeerSyncReport }
  'peer.allow': { args: [peer: string, grant: PeerGrant]; result: PeerGrant }
  'peer.route': { args: [peer: string, ssh: string | null, addrs: string[]]; result: true }
  'peer.remove': { args: [peer: string]; result: true }
  'peer.serve': { args: [on: boolean]; result: { listening: string | null } }
  'peer.standby': { args: [peer: string, sessionId: string, on: boolean]; result: true }
  'acp.setup': { args: []; result: AcpSetup }
  'acp.agents': { args: []; result: AcpAgentRow[] }
  'acp.run': { args: [agent: string, prompt: string]; result: AcpRunResult }
  'acp.answer': { args: [id: number, optionId: string | null]; result: true }
  'privacy.cards': { args: []; result: PrivacyCards }
  'privacy.setRequire': { args: [require: string[]]; result: string[] }
  'chromeExtension.pair': { args: []; result: { code: string; expires: number } }
  'chromeExtension.revoke': { args: []; result: void }
  'chromeExtension.domains': { args: []; result: ChromeDomains }
  'chromeExtension.allow': { args: [domain: string]; result: ChromeDomains }
  'chromeExtension.deny': { args: [domain: string]; result: ChromeDomains }
  'chromeExtension.forget': { args: [domain: string]; result: ChromeDomains }
  'chromeExtension.install': { args: [browser?: string, openFolder?: boolean]; result: { installed: { browser: string; manifest: string }[] } }
  'chromeExtension.uninstall': { args: []; result: { removed: string[] } }
  'chromeExtension.setBackend': { args: [backend: string]; result: void }
  'chromeExtension.profiles': { args: []; result: string[] }
  'chromeExtension.profileGet': { args: [name: string]; result: Record<string, string> }
  'chromeExtension.profileSet': { args: [name: string, values: Record<string, string>]; result: void }
  'chromeExtension.profileRemove': { args: [name: string]; result: boolean }
  'chromeExtension.docsCache': { args: []; result: { url: string; title: string; fetchedAt: number }[] }
  'chromeExtension.docsCacheClear': { args: []; result: number }
  'fork.messages': { args: [id: string]; result: unknown[] }
  'fork.create': { args: [id: string, at: number, model: string | null, noWorktree: boolean]; result: {
      id: string
      parent: string
      at: number
      worktree?: string | null
      branch?: string | null
      rewound?: string[]
      unrestored?: string[]
      shellAfter?: boolean
    } }
  'fork.tree': { args: [id?: string | null]; result: unknown[] }
  'fork.compare': { args: [a: string, b: string]; result: object }
  'fork.mergePlan': { args: [id: string]; result: object }
  'fork.mergeApply': { args: [id: string, files: unknown[], note: boolean]; result: object }
  'fork.remove': { args: [id: string, worktree: boolean]; result: null }
  'diagram.list': { args: []; result: unknown[] }
  'diagram.sync': { args: [force: boolean]; result: object }
  'diagram.add': { args: [file: string, kind: string, format: string, params: object]; result: object }
  'know.search': { args: [query: string, here: boolean]; result: unknown[] }
  'know.folder': { args: []; result: { path: string; written: number; removed: number } }
  'know.digest': { args: [week?: string | null]; result: object }
  'know.digests': { args: []; result: unknown[] }
  'know.stale': { args: [here: boolean]; result: unknown[] }
  'fim.complete': { args: [file: string, line: number, col?: number, write?: boolean]; result: { text: string; written: boolean } }
  'estimate.task': { args: [text: string, sessionId: string | null, model: string | null]; result: unknown }
  'rules.check': { args: [staged: boolean, judge: boolean]; result: object }
  'rules.list': { args: []; result: unknown[] }
  'rules.report': { args: [days: number]; result: unknown[] }
  'docs.list': { args: []; result: unknown[] }
  'docs.add': { args: [source: string, browser: boolean]; result: object }
  'docs.pick': { args: []; result: unknown }
  'docs.search': { args: [query: string]; result: unknown[] }
  'docs.refresh': { args: [id?: string | null]; result: unknown[] }
  'docs.remove': { args: [id: string]; result: null }
  'hooks.events': { args: []; result: object }
  'hooks.files': { args: []; result: unknown[] }
  'hooks.list': { args: []; result: object }
  'hooks.save': { args: [fileIndex: number, item: object, replace: unknown]; result: null }
  'hooks.remove': { args: [fileIndex: number, event: string, id: string]; result: null }
  'hooks.test': { args: [fileIndex: unknown, item: object, sample: object, execute: boolean]; result: object }
  'hooks.log': { args: [limit: number]; result: unknown[] }
  'mcpdev.start': { args: [path: string, command: string | null]; result: object }
  'mcpdev.list': { args: []; result: unknown[] }
  'mcpdev.inspect': { args: [id: number]; result: object }
  'mcpdev.reload': { args: [id: number]; result: object }
  'mcpdev.call': { args: [id: number, tool: string, argsJson: string]; result: unknown }
  'mcpdev.read': { args: [id: number, uri: string]; result: unknown }
  'mcpdev.prompt': { args: [id: number, name: string, argsJson: string]; result: unknown }
  'mcpdev.log': { args: [id: number]; result: unknown[] }
  'mcpdev.stop': { args: [id: number]; result: boolean }
  'computer.resetTier': { args: [app: string]; result: ComputerStatus }
  'computer.setEnabled': { args: [on: boolean]; result: ComputerStatus }
  'computer.setTier': { args: [app: string, tier: ComputerTier]; result: ComputerStatus }
  'computer.status': { args: []; result: ComputerStatus }
  'computer.stop': { args: []; result: ComputerStatus }
  'layout.delete': { args: [name: string]; result: null }
  'layout.linked': { args: []; result: string | null }
  'layout.list': { args: []; result: { layouts: WindowLayout[]; linked: string | null } }
  'layout.restore': { args: [name: string]; result: LayoutPlan }
  'layout.save': { args: [name: string, linkProject: boolean]; result: WindowLayout }
  'system.clean': { args: [ids: string[]]; result: CleanOutcome[] }
  'system.cleanSuggest': { args: []; result: CleanCandidate[] }
  'system.health': { args: [deep: boolean]; result: HealthAlert[] }
  'system.exportAll': { args: []; result: { path: string; files: number; bytes: number } }
  'system.mute': { args: [kind: string]; result: string[] }
  'system.profile': { args: []; result: SystemProfile }
  'system.refresh': { args: []; result: SystemRefresh }
}

/** Payload of every `kor:*` event. */
export interface IpcEvents {
  'kor:abrowser/event': BrowserPanelEvent
  'kor:bench/done': BenchDone
  'kor:bench/progress': BenchProgress
  'kor:deploy/done': DeployReport
  'kor:deploy/event': { env: string; event: DeployEvent }
  'kor:event': AgentEvent
  'kor:g14/alert': G14Alert
  'kor:g16/updateAvailable': { version: string | null; signed: boolean }
  'kor:g30/pending': void
  'kor:g4/event': G4Event
  'kor:graph/touched': GraphTouchedEvent
  'kor:graph/updated': GraphUpdatedEvent
  'kor:korfolder/changed': KorFolderPass
  'kor:media/cancelled': { jobId: string }
  'kor:media/hfProgress': HfProgress
  'kor:media/progress': MediaProgress
  'kor:offline/done': OfflineDone
  'kor:offline/progress': OfflineEvent
  'kor:osint/request': OsintRequest
  'kor:plugins/installProgress': { name: string; percent: number }
  'kor:prereq/progress': PrereqProgress
  'kor:preview/event': PreviewEvent
  'kor:remote/closed': { target: string }
  'kor:remote/event': RemoteEvent
  'kor:runtimeProgress': InstallTick & { id: string }
  'kor:seo/agent': SeoAgentEvent
  'kor:seo/oauth': { url: string }
  'kor:proxy/oauth': {
    kind: 'browser' | 'device'
    url: string
    userCode?: string
    verificationUri?: string
    verificationUriComplete?: string | null
  }
  'kor:seo/progress': SeoProgress
  'kor:sessionsChanged': void
  'kor:shellui/answered': { sessionId: string; id: string }
  'kor:shellui/changed': void
  'kor:shellui/newChat': void
  'kor:shellui/notification': { notification: KorNotification; alert: Alert }
  'kor:shellui/openPanel': string
  'kor:shellui/openProject': string
  'kor:shellui/openSession': string
  'kor:shellui/openUpdate': void
  'kor:tasks/progress': TaskState
  'kor:watch/event': { id: number; event: WatchEvent }
  'kor:window/closeRequested': unknown
  'kor:agent/status': AgentStatusEvent
  'kor:context/compactPreview': CompactPreviewEvent
  'kor:g9/pullProgress': PullState
  'kor:plan/approved': PlanInfo
  'kor:media/done': { jobId: string; paths: string[]; items?: MediaGalleryItem[]; report?: string }
  'kor:peer/event': PeerEventPayload
  'kor:peer/paired': { name: string }
  'kor:peer/handoff': PeerHandoff
  'kor:acp/update': AcpUpdate
  'kor:acp/permission': AcpPermission
  'kor:mcpdev/traffic': object
  'kor:mcpdev/reloaded': object
  'kor:computer/activity': ComputerActivity
  'kor:layout/restored': { name: string; plan: LayoutPlan }
  'kor:system/changed': SystemRefresh
  'kor:system/health': HealthAlert
}

export const IPC_METHODS = ["abort","abrowser.activate","abrowser.back","abrowser.close","abrowser.forward","abrowser.hide","abrowser.logs","abrowser.navigate","abrowser.newTab","abrowser.reload","abrowser.setBounds","abrowser.show","abrowser.tabs","activateSession","agents.list","analysis.compliance","analysis.export","analysis.security","analysis.verifyReport","approve","audit.export","audit.list","audit.report","audit.verify","batch.apply","batch.cancel","batch.plan","bench.cancel","bench.estimate","bench.result","bench.start","buildProjectMap","cancelProjectMap","catalog.get","chromeExtension.info","chromeExtension.openFolder","clearWorkspace","commands.list","compactSession","compat.conflicts","compat.export","compat.mcp","compat.reveal","components.add","components.list","components.installAll","components.update","components.verify","components.remove","connections.list","connections.remove","connections.saveDb","connections.saveSsh","connections.testDb","connections.testSsh","connections.trustHostKey","context.history","context.last","core.addMarketplace","core.compat","core.configExplain","core.configSet","core.marketplaces","core.pluginDoctor","core.pluginSearch","core.pluginCreate","core.pluginRevoke","core.pluginRun","core.privacy","core.removeMarketplace","core.setPrivacy","core.undo","core.undoList","core.updateChannel","cost.export","cost.exportFile","cost.quota","cost.settings","cost.summary","cost.why","createSection","db.apply","db.check","db.indexes","db.migrateNew","db.trial","deleteSection","deleteSession","deploy.envs","deploy.plan","deploy.risk","deploy.run","deps.check","deps.licenses","deps.list","duplicateSession","editCompactionSummary","env.status","exportSession","find.run","forge.detect","forge.personas","forge.prCreate","forge.prDescribe","forge.prList","forge.prRespond","forge.prRespondApply","forge.prReview","forge.prReviewPost","forge.prStatus","forge.review","g10.dismiss","g10.installGit","g10.listDeadLetters","g10.listRecoverable","g10.recoveryInfo","g10.removeDeadLetter","g10.restore","g10.retryDeadLetter","g10.selfCheck","g11Forget","g11GetOverview","g11KeepBoth","g11SetConfidence","g12AnswerBulkDelete","g12GetSandbox","g12GetToolScopes","g12SetSandbox","g12SetToolScope","g13.buildExport","g13.envCreateTemplate","g13.envFiles","g13.envKeys","g13.envRemove","g13.envReveal","g13.envSet","g13.envTemplateReport","g13.getDocSources","g13.getLayers","g13.saveExport","g13.setDocSources","g13.setProjectPrompt","g13.setSessionPrompt","g14.cost","g14.getSettings","g14.getTags","g14.overview","g14.probe","g14.processes","g14.rate","g14.raw","g14.setSettings","g14.setTags","g14.status","g16CliInstall","g16CliRemove","g16CliStatus","g16ConfigExport","g16ConfigImport","g16CrashExport","g16CrashList","g16CrashOpenFolder","g16CrashSend","g16UpdateCheck","g16UpdateInstall","g16UpdateStatus","g17ExportPdf","g20ReadNotebook","g20RunPython","g20RunSql","g20StripTs","g21ReadDoc","g22SetToolMode","g22Support","g27InstallRuntime","g27ListRuntimes","g2EditMessage","g2ModelCaps","g2PathForFile","g2QueueClear","g2QueueFlush","g2QueueRemove","g2Regenerate","g2ResolveDropped","g2SearchFiles","g2Send","g2StatFiles","g30.getAutoMemory","g30.pendingList","g30.pendingResolve","g30.setAutoMemory","g35.login","g35.logout","g35.restore","g35.status","g35.submitCode","g3Answer","g3GetOutputStyle","g3LinkPreview","g3RunCode","g3SaveCode","g3SetOutputStyle","g4.createDir","g4.createFile","g4.detectEditors","g4.listAllFiles","g4.listCheckpoints","g4.listDir","g4.openInEditor","g4.preview","g4.previewRewind","g4.rename","g4.rewind","g4.runDiagnostics","g4.trash","g4.watch","g4.writeFile","g7AbortProbe","g7AddPermissionRule","g7CheckOnline","g7ClearData","g7GetCatalog","g7GetPermissionRules","g7GetPrivacyInfo","g7GetSettings","g7GetTrust","g7GetUsageHistory","g7OpenFolder","g7RemovePermissionRule","g7RunProbe","g7SetEditor","g7SetGit","g7SetLogs","g7SetModelAliases","g7SetNotifications","g7SetProfiles","g7SetScripts","g7SetTaskModels","g7SetTheme","g7SetTrust","g9.addKey","g9.clearResponseCache","g9.costInfo","g9.getProviders","g9.health","g9.ollamaCancelPull","g9.ollamaList","g9.ollamaPull","g9.ollamaUnload","g9.removeKey","g9.saveSettings","g9.setActiveKey","g9.setEndpoint","g9.setKeyBackup","g9.testKey","getCompactSettings","getContextInfo","getFileChange","getLastSession","getMemory","getModelPricing","getProjectMap","getSessionConfig","getState","getSystemPrompt","getUsage","git.abortOperation","git.branches","git.continueOperation","git.createPr","git.merge","git.openPr","git.push","git.rebase","git.status","gitflow.changelog","gitflow.commit","gitflow.commitMessage","graph.diff","graph.explain","graph.export","graph.hotspots","graph.impact","graph.path","graph.query","graph.reveal","graph.snapshot","graph.status","graph.update","initProject","intent.forget","intent.learn","intent.list","intent.route","intent.run","intent.schedule","korfolder.disable","korfolder.enable","korfolder.mount","korfolder.open","korfolder.status","korfolder.sync","korfolder.unmount","listDir","listModels","listSections","listSessions","liveSessions","loadSessionView","logs.list","mcp.list","mcp.reconnect","mcp.remove","mcp.save","mcp.setEnabled","mcpgen.add","mcpgen.test","media.cancel","media.hfDownload","media.hfInfo","media.hfSearch","media.jobs","memory.decisions","memory.export","memory.forget","memory.link","memory.notesPull","memory.notesPush","memory.projects","memory.recall","memory.reindex","memory.semantic","memory.unlink","memory.workspaceAdd","memory.workspaceRemove","memory.workspaces","metrics.deadCode","metrics.deadCodePrune","metrics.deadCodePublish","metrics.debt","cost.consensus","cost.shadow","metrics.comment","metrics.explain","metrics.impact","metrics.quality","metrics.qualityHistory","metrics.stats","metrics.style","metrics.styleCheck","moveSessionToSection","notes.get","notes.set","offline.bundle","offline.import","offline.init","offline.plan","offline.setEnabled","offline.status","openExternal","openSession","openSessionWindow","openWorkspace","osextra.handoff","osextra.install","osextra.list","osextra.portals","osextra.remoteClipboard","osextra.spotlightSync","osextra.uninstall","osint.drop","osint.hotkeyGet","osint.hotkeySet","osint.menuInstall","osint.menuStatus","osint.menuUninstall","osint.pending","osint.recent","packages.affected","packages.cost","packages.deps","packages.list","perf.list","perf.load","perf.optimizePrompt","perf.report","perf.run","pickFiles","pinSession","plan.approve","plan.list","plan.show","plugins.addMcp","plugins.catalog","plugins.checkUpdates","plugins.install","plugins.installRepo","plugins.list","plugins.openFolder","plugins.setEnabled","plugins.storeSearch","plugins.uninstall","plugins.update","prefs.agent","prefs.answer","prefs.auto","prefs.doctorFix","prefs.keymap","prefs.setAgent","prefs.setAuto","prefs.setUi","prefs.ui","prereq.install","prereq.list","preview.back","preview.devStart","preview.devStatus","preview.devStop","preview.hide","preview.navigate","preview.openExternal","preview.reload","preview.setBounds","preview.setInspect","preview.show","projmem.addDecision","projmem.overview","projmem.pickFolder","projmem.read","projmem.recent","projmem.session","projmem.write","provider.status","provider.test","proxy.config","proxy.key","proxy.start","proxy.status","proxy.stop","proxy.oauthAccounts","proxy.oauthLogin","proxy.oauthRefresh","proxy.oauthRemove","proxy.oauthSetMeta","proxy.routing","proxy.routingSet","readFile","remote.abort","remote.approve","remote.attach","remote.connect","remote.disconnect","remote.discover","remote.hosts","remote.report","remote.send","remote.status","remote.tasks","renameSection","renameSession","replay.export","replay.list","replay.load","revertFile","routines.add","routines.list","routines.remove","routines.setEnabled","sandbox.pull","sandbox.setNetwork","sandbox.status","sandbox.test","send","seo.agents","seo.agentsSchedule","seo.audit","seo.auditSchedule","seo.bingSetup","seo.bingStats","seo.bingStatus","seo.competitorsAdd","seo.competitorsList","seo.competitorsRemove","seo.competitorsRun","seo.competitorsSchedule","seo.contentApprove","seo.contentList","seo.contentNew","seo.contentPublish","seo.contentRead","seo.contentRun","seo.contentSave","seo.exportReport","seo.gaReport","seo.gscDrops","seo.gscDropsSchedule","seo.gscLogin","seo.gscLogout","seo.gscQuery","seo.gscSetup","seo.gscSites","seo.gscStatus","seo.logs","seo.metaApply","seo.metaCollect","seo.metaEstimate","seo.metaExport","seo.metaGenerate","seo.metaImport","seo.metaUndo","seo.metaValidate","seo.opportunities","seo.verify","sessions.archive","sessions.color","sessions.export","sessions.import","sessions.importFrom","sessions.list","sessions.mv","sessions.pin","sessions.rename","sessions.search","sessions.suggest","sessions.tag","setActiveSession","setApiKey","setCompactSettings","setDefaultPermissionMode","setLastSession","setMemory","setModel","setPermissionMode","setSessionArchived","setSessionConfig","setSystemPrompt","setUiLanguage","setWorkspacePath","shell.fix","shell.install","shell.runFix","shell.status","shell.uninstall","shell.why","shellui.activity","shellui.answerMany","shellui.approve","shellui.deny","shellui.hidePopup","shellui.notifications","shellui.notificationsClear","shellui.notificationsRead","shellui.openMain","shellui.openProject","shellui.pause","shellui.pip","shellui.queue","shellui.quickTask","shellui.setSetting","shellui.settings","shellui.state","shellui.status","shellui.stop","shellui.updateOnExit","tags.list","tags.save","tasks.cancel","tasks.fork","tasks.list","tasks.resolve","tasks.resume","tasks.start","tasks.status","team.check","team.connect","team.disconnect","team.status","teams.list","teams.saveUser","templates.draft","templates.list","templates.new","templates.run","templates.show","terminal.run","tools.add","tools.describe","tools.list","tools.pull","tools.search","tools.share","triggers.answer","triggers.daemonInstall","triggers.daemonLogs","triggers.daemonStart","triggers.daemonStatus","triggers.daemonStop","triggers.daemonUninstall","triggers.history","triggers.hookInstall","triggers.hookUninstall","triggers.list","triggers.notifyGet","triggers.notifySet","triggers.notifyTest","triggers.remove","triggers.save","triggers.test","vault.add","vault.import","vault.list","vault.remove","visual.diagram","visual.screenshot","visual.visualize","visual.voice","visual.voiceStatus","visual.write","watch.clear","watch.fixFiles","watch.fixFinish","watch.fixStart","watch.incidents","watch.list","watch.start","watch.stop","web.businessInit","web.clientAdd","web.clientCheck","web.clientLog","web.clientReport","web.clients","web.detect","web.hostConnect","web.hostPlan","web.hostRemove","web.hostTask","web.hosts","web.inventory","web.proposal","web.proposalRevise","web.redirectsBuild","web.redirectsCheck","web.redirectsExport","web.redirectsSlugs","web.sites","web.templates","web.ticket","web.trCheck","web.trEfatura","web.trIrsaliye","web.trKdv","web.trKeywords","web.trKvkk","web.trMarket","web.trPayment","web.trSlug","web.wpCleanup","web.wpConnect","web.wpDoctor","web.wpHealth","web.wpIntegrity","web.wpInventory","web.wpRemove","web.wpScaffold","web.wpStaging","web.wpUpdate","webview.closeNow","webview.devtools","webview.sampleProject","webview.saveImage","catalog.hooks","provider.add","provider.remove","provider.discover","provider.keys","provider.addKey","provider.removeKey","media.falModels","media.falRun","media.falImage","media.hfInfer","shell.capture","db.connect","db.refresh","db.tables","db.show","context.items","context.drop","context.pin","core.ipcTrace","perfmon.config","perfmon.stats","perfmon.log","perfmon.ready","perfmon.idle","perfmon.benchSearch","perfmon.benchDone","perfmon.benchError","perfmon.causes","media.catalog","media.plan","media.generate","media.gallery","media.item","media.export","media.pending","media.collect","media.pasteRef","media.importRef","peer.list","peer.pairCode","peer.join","peer.run","peer.attach","peer.approve","peer.abort","peer.tasks","peer.result","peer.apply","peer.sync","peer.allow","peer.route","peer.remove","peer.serve","peer.standby","acp.setup","acp.agents","acp.run","acp.answer","privacy.cards","privacy.setRequire","chromeExtension.pair","chromeExtension.revoke","chromeExtension.domains","chromeExtension.allow","chromeExtension.deny","chromeExtension.forget","chromeExtension.install","chromeExtension.uninstall","chromeExtension.setBackend","chromeExtension.profiles","chromeExtension.profileGet","chromeExtension.profileSet","chromeExtension.profileRemove","chromeExtension.docsCache","chromeExtension.docsCacheClear","fork.messages","fork.create","fork.tree","fork.compare","fork.mergePlan","fork.mergeApply","fork.remove","diagram.list","diagram.sync","diagram.add","know.search","know.folder","know.digest","know.digests","know.stale","fim.complete","estimate.task","rules.check","rules.list","rules.report","docs.list","docs.add","docs.pick","docs.search","docs.refresh","docs.remove","hooks.events","hooks.files","hooks.list","hooks.save","hooks.remove","hooks.test","hooks.log","mcpdev.start","mcpdev.list","mcpdev.inspect","mcpdev.reload","mcpdev.call","mcpdev.read","mcpdev.prompt","mcpdev.log","mcpdev.stop","computer.resetTier","computer.setEnabled","computer.setTier","computer.status","computer.stop","layout.delete","layout.linked","layout.list","layout.restore","layout.save","system.clean","system.cleanSuggest","system.health","system.exportAll","system.mute","system.profile","system.refresh"] as const

export const IPC_EVENTS = ["kor:abrowser/event","kor:bench/done","kor:bench/progress","kor:deploy/done","kor:deploy/event","kor:event","kor:g14/alert","kor:g16/updateAvailable","kor:g30/pending","kor:g4/event","kor:graph/touched","kor:graph/updated","kor:korfolder/changed","kor:media/cancelled","kor:media/hfProgress","kor:media/progress","kor:offline/done","kor:offline/progress","kor:osint/request","kor:plugins/installProgress","kor:prereq/progress","kor:preview/event","kor:remote/closed","kor:remote/event","kor:runtimeProgress","kor:seo/agent","kor:seo/oauth","kor:proxy/oauth","kor:seo/progress","kor:sessionsChanged","kor:shellui/answered","kor:shellui/changed","kor:shellui/newChat","kor:shellui/notification","kor:shellui/openPanel","kor:shellui/openProject","kor:shellui/openSession","kor:shellui/openUpdate","kor:tasks/progress","kor:watch/event","kor:window/closeRequested","kor:agent/status","kor:context/compactPreview","kor:g9/pullProgress","kor:plan/approved","kor:media/done","kor:peer/event","kor:peer/paired","kor:peer/handoff","kor:acp/update","kor:acp/permission","kor:mcpdev/traffic","kor:mcpdev/reloaded","kor:computer/activity","kor:layout/restored","kor:system/changed","kor:system/health"] as const
