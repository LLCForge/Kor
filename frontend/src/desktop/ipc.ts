// Arayüz (window.kor) ile Rust arka ucu (src-tauri, `kor_call`) arasındaki IPC sözleşmesi.
import type { Approval, RateLimitWindow } from '../core/agent.ts'
import type { PermissionMode } from '../core/permissions.ts'
import type { SessionMeta, TranscriptItem } from '../core/sessions.ts'

export type { SessionMeta, TranscriptItem }

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

export interface SubagentEventInfo {
  agentType: string
  description: string
  background: boolean
  model: string
}

/** Her olay hangi sohbete ait olduğunu taşır: birden fazla sohbet aynı anda çalışabilir. */
export type AgentEventBody =
  | { type: 'turnStart' }
  | { type: 'text'; delta: string }
  /** parentId: olay bir alt ajanın aracına aitse, o alt ajanı başlatan agent çağrısının kimliği */
  | { type: 'tool'; id: string; name: string; summary: string; parentId?: string }
  | { type: 'toolResult'; id: string; result: string }
  /** Çalışan aracın canlı satırı: önce `$ komut`, sonra çıktı (Bash kartı) */
  | { type: 'toolProgress'; id: string; line: string }
  /** preview: dosya araçlarında onaydan önce hesaplanan değişiklik; onay kartı gerçek diff'i gösterir */
  | { type: 'confirm'; id: string; name: string; summary: string; warning?: string; preview?: { path: string; before: string | null; after: string } }
  /** before: oturumdaki ilk hal (diff paneli), previous: bu düzenlemeden hemen önceki hal (+/- sayımı) */
  | { type: 'fileChanged'; path: string; before: string | null; previous: string | null; after: string; toolId?: string; /** [perf] büyük dosya: içerik gönderilmedi, istatistik ana süreçte hesaplandı; içerik getFileChange ile alınır */ large?: { added: number; removed: number; bytes: number } }
  | ({ type: 'subagent'; id: string } & SubagentEventInfo)
  | { type: 'subagentDone'; id: string; result: string; ok: boolean }
  | { type: 'notice'; text: string }
  | { type: 'done' }
  | { type: 'error'; message: string }

// [g1] yeni olay türleri (| { type: ... })

// [g2] yeni olay türleri (| { type: ... })
  /** Kuyruktaki mesajlar konuşmaya eklendi: arayüz "Sırada" etiketini kaldırıp kullanıcı mesajı olarak gösterir */
  | { type: 'g2:queueConsumed'; items: Array<{ id: string; text: string; attachments: string[] }> }

// [g3] yeni olay türleri (| { type: ... })
  /** Akıl yürütme modelinin düşünme metni (parça) */
  | { type: 'g3:reasoning'; delta: string }
  /** Oturumun modeli değişti */
  | { type: 'g3:model'; from: string; to: string }

// [g4] yeni olay türleri (| { type: ... })

// [g5] yeni olay türleri (| { type: ... })

// [g6] yeni olay türleri (| { type: ... })
  | { type: 'g6:context'; info: import('./ipc-g6.ts').ContextInfo }
  | ({ type: 'g6:compacted' } & import('./ipc-g6.ts').CompactionEvent)
  | { type: 'g6:compactionState'; running: boolean }
  | { type: 'g6:compactionFailed'; message: string }

// [g9] yeni olay türleri (| { type: ... })
  /** Görev (tur + alt ajanlar) maliyeti; akış sürerken tahmini */
  | ({ type: 'g9:cost' } & import('./ipc-g9.ts').TurnCostInfo)

// [g10] yeni olay türleri (| { type: ... })
  /** Güvenilirlik olayı: devre kesici, takılan araç, yeniden bağlanma, yedek model, JSON onarımı, ret, çakışma, ölü mektup */
  | { type: 'g10:event'; event: import('./ipc-g10.ts').G10Event }

// [g11] yeni olay türleri (| { type: ... })

// [g12] yeni olay türleri (| { type: ... })
  /** Toplu silme için dosya listeli onay bekleniyor; yanıt g12AnswerBulkDelete ile */
  | { type: 'g12:bulkDelete'; id: string; request: import('./ipc-g12.ts').BulkDeleteRequest }
  /** Onay yanıtlandı veya tur iptal edildi */
  | { type: 'g12:bulkDeleteDone'; id: string }

// [g13] yeni olay türleri (| { type: ... })

// [g14] yeni olay türleri (| { type: ... })

// [g15] yeni olay türleri (| { type: ... })

// [g16] yeni olay türleri (| { type: ... })

// [g18] yeni olay türleri (| { type: ... })

// [g19] yeni olay türleri (| { type: ... })

// [g20] yeni olay türleri (| { type: ... })

// [g21] yeni olay türleri (| { type: ... })

// [g22] yeni olay türleri (| { type: ... })

// [g7] yeni olay türleri (| { type: ... })

export type AgentEvent = AgentEventBody & { sessionId: string }

/** Ana süreçte süren sohbet: yeniden yüklenen arayüz çalışma durumunu ve bekleyen kartları bundan kurar. */
export interface LiveSession {
  sessionId: string
  /** Tur başlangıcı (epoch ms); null: yalnızca arka plan alt ajanı sürüyor */
  since: number | null
  events: AgentEventBody[]
}

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

  // [g1] AppState alanları

  // [g2] AppState alanları

  // [g3] AppState alanları

  // [g4] AppState alanları

  // [g5] AppState alanları

  // [g6] AppState alanları

  // [g9] AppState alanları

  // [g10] AppState alanları

  // [g11] AppState alanları

  // [g12] AppState alanları

  // [g13] AppState alanları

  // [g14] AppState alanları

  // [g15] AppState alanları

  // [g16] AppState alanları

  // [g18] AppState alanları

  // [g19] AppState alanları

  // [g20] AppState alanları

  // [g21] AppState alanları

  // [g22] AppState alanları

  // [g7] AppState alanları
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

export interface SystemPromptInfo {
  /** Kullanıcının kaydettiği istem; null ise varsayılan kullanılıyor */
  custom: string | null
  defaultInstructions: string
}

export interface KorApi {
  getState(): Promise<AppState>
  /** Arayüz ve arka uç mesajlarının dilini kullanıcı `config.toml` dosyasına yazar. */
  setUiLanguage(lang: 'auto' | 'tr' | 'en' | 'fr'): Promise<void>
  setApiKey(key: string): Promise<void>
  openWorkspace(): Promise<string | null>
  /** Son kullanılan bir klasöre diyalog açmadan geçer. */
  setWorkspacePath(path: string): Promise<string | null>
  /** Klasörü kaldırır; sohbet varsayılan çalışma alanında devam eder. */
  clearWorkspace(): Promise<void>
  listDir(dir: string): Promise<FileEntry[]>
  readFile(path: string): Promise<string>
  listModels(): Promise<ModelInfo[]>
  setModel(model: string): Promise<void>
  setPermissionMode(mode: PermissionMode): Promise<void>
  setDefaultPermissionMode(mode: PermissionMode): Promise<void>
  getUsage(sessionId: string | null): Promise<UsageInfo>
  listSessions(): Promise<SessionMeta[]>
  /** Kayıtlı veya bellekte çalışan sohbeti açar; arayüz bu sohbetin durumunu tutmuyorsa döküm döner */
  openSession(id: string): Promise<TranscriptItem[]>
  liveSessions(): Promise<LiveSession[]>
  deleteSession(id: string): Promise<void>
  pinSession(id: string, pinned: boolean): Promise<void>
  /** Bildirimlerin doğru yönlendirilmesi için arayüzde görünen sohbet; null = yeni sohbet taslağı */
  setActiveSession(id: string | null): Promise<void>
  getMemory(): Promise<string>
  setMemory(text: string): Promise<void>
  /** Turu başlatır ve hemen döner; ilerleme olaylarla gelir. sessionId yeni sohbet için arayüzün ürettiği UUID'dir. */
  /** openFile: yan panelde açık dosya (otomatik bağlam); null: açık dosya yok, verilmezse değişmez */
  send(sessionId: string, text: string, attachments: string[], openFile?: string | null): Promise<void>
  abort(sessionId: string): Promise<void>
  approve(id: string, approval: Approval): Promise<void>
  revertFile(sessionId: string, path: string): Promise<void>
  getSystemPrompt(): Promise<SystemPromptInfo>
  setSystemPrompt(text: string): Promise<void>
  /** Çalışma dizininden dosya seçtirir, göreli yolları döner */
  pickFiles(): Promise<string[]>
  openExternal(url: string): Promise<void>

  // [g1] API yöntemleri
  /** null veya boş: kullanıcı adı kaldırılır, otomatik başlığa dönülür */
  renameSession(id: string, title: string | null): Promise<void>
  setSessionArchived(id: string, archived: boolean): Promise<void>
  /** null: bölümden çıkarır (proje grubuna döner) */
  moveSessionToSection(id: string, sectionId: string | null): Promise<void>
  /** Yan bölme için: sohbeti etkin yapmadan belleğe yükler ve dökümünü döner */
  loadSessionView(id: string): Promise<TranscriptItem[]>
  getSessionConfig(id: string): Promise<{ model: string; permissionMode: PermissionMode }>
  setSessionConfig(id: string, config: { model?: string; permissionMode?: PermissionMode }): Promise<void>
  /** Pencere odaklanınca: bellekteki sohbeti yeniden etkin yapar; durum değiştiyse true */
  activateSession(id: string): Promise<boolean>
  openSessionWindow(id: string): Promise<void>
  /** [cdx] Sohbeti kopyalar (spinOff: 'dal' olarak adlandırır); yeni kimliği döner */
  duplicateSession(id: string, spinOff: boolean): Promise<string>
  /** [perf] Büyük dosya değişikliğinin tam içeriği (fileChanged.large geldiğinde isteğe bağlı) */
  getFileChange(sessionId: string, path: string): Promise<{ before: string | null; after: string } | null>
  /** [cdx] Sohbeti Markdown dosyasına dışa aktarır (kaydetme penceresi); iptalde null, yoksa yazılan yol */
  exportSession(id: string): Promise<string | null>
  listSections(): Promise<import('../core/sections.ts').Section[]>
  createSection(name: string): Promise<import('../core/sections.ts').Section>
  renameSection(id: string, name: string): Promise<void>
  deleteSection(id: string): Promise<void>
  /** Geri yüklenebilecek son açık sohbet (dosyası ve klasörü varsa) */
  getLastSession(): Promise<string | null>
  setLastSession(id: string | null): Promise<void>
  /** USD / 1M token, model listesi alındıktan sonra dolar */
  getModelPricing(): Promise<Record<string, { input: number; output: number }>>
  /** Başka pencereden/arka plandan sohbet listesi veya bölümler değişince */
  onSessionsChanged(listener: () => void): () => void

  // [g2] API yöntemleri
  /** Boştaysa turu başlatır, çalışıyorsa kuyruğa alır (interrupt: turu kesip hemen başlatır) */
  g2Send(sessionId: string, payload: import('./ipc-g2.ts').G2SendPayload): Promise<import('./ipc-g2.ts').G2SendResult>
  /** Henüz tüketilmediyse kuyruktan çıkarır; çıkarıldıysa true */
  g2QueueRemove(sessionId: string, queueId: string): Promise<boolean>
  /** Kuyruğu boşaltır, çıkarılan kimlikleri döner */
  g2QueueClear(sessionId: string): Promise<string[]>
  /** Tur bittiğinde kuyrukta kalan mesaj varsa yeni tur başlatır */
  g2QueueFlush(sessionId: string): Promise<void>
  /** Çalışma dizininde bulanık dosya/klasör araması (en fazla 50) */
  g2SearchFiles(query: string): Promise<import('./ipc-g2.ts').G2FileHit[]>
  g2StatFiles(paths: string[]): Promise<import('./ipc-g2.ts').G2FileStat[]>
  /** Sürüklenen mutlak yolları çalışma dizinine göre doğrular */
  g2ResolveDropped(absPaths: string[]): Promise<import('./ipc-g2.ts').G2DropResult>
  g2ModelCaps(): Promise<Record<string, import('./ipc-g2.ts').G2ModelCaps>>
  g2EditMessage(sessionId: string, payload: import('./ipc-g2.ts').G2EditPayload): Promise<void>
  g2Regenerate(sessionId: string, payload: import('./ipc-g2.ts').G2RegeneratePayload): Promise<void>
  /** Sürüklenen dosyanın yerel yolu (Electron webUtils, IPC değil); yol yoksa boş dize */
  g2PathForFile(file: File): string

  // [g3] API yöntemleri
  /** Bekleyen ask_user / exit_plan_mode kartını cevaplar */
  g3Answer(sessionId: string, callId: string, answer: import('../core/interactive.ts').InteractAnswer): Promise<void>
  g3GetOutputStyle(): Promise<import('../core/output-styles.ts').OutputStyle>
  g3SetOutputStyle(style: import('../core/output-styles.ts').OutputStyle): Promise<void>
  /** Yalnızca genel (public) http(s) adresleri; null = önizlenecek bilgi yok */
  g3LinkPreview(url: string): Promise<import('../core/link-preview.ts').LinkPreview | null>
  /** Kaydetme penceresi açar; kaydedilen göreli yol veya iptalde null */
  g3SaveCode(content: string, language: string): Promise<string | null>
  /** Yerel onay penceresinden sonra çalıştırır; iptalde null */
  g3RunCode(code: string, language: string): Promise<import('../core/snippet-runner.ts').SnippetResult | null>

  // [g4] API yöntemleri
  /** Dosya ağacı, önizleme, editör, dosya işlemleri, izleme, kontrol noktaları, tanılama (bkz. ipc-g4.ts) */
  g4: import('./ipc-g4.ts').G4Api

  // [g5] API yöntemleri

  // [g6] API yöntemleri
  /** Eski turları özetler (/compact); sohbet çalışırken reddedilir */
  compactSession(sessionId: string): Promise<import('./ipc-g6.ts').CompactionResult>
  /** Sıkıştırma özetini kullanıcı düzeltmesiyle değiştirir */
  editCompactionSummary(sessionId: string, body: string): Promise<boolean>
  /** /init: AGENTS.md taslağı; dosya varsa overwrite=false ile yalnızca {status:'exists'} döner */
  initProject(overwrite: boolean): Promise<import('./ipc-g6.ts').InitProjectResult>
  getProjectMap(): Promise<import('./ipc-g6.ts').ProjectMapInfo>
  /** Arka planda üretir/günceller; ilerleme getProjectMap ile izlenir. force: tüm alanlar baştan */
  buildProjectMap(force: boolean): Promise<void>
  cancelProjectMap(): Promise<void>
  /** sessionId null: yeni sohbet taslağı (yalnızca seçili modelin sınırı) */
  getContextInfo(sessionId: string | null): Promise<import('./ipc-g6.ts').ContextView>
  /** Otomatik sıkıştırma eşiği ayarları (tavan token ve oran) */
  getCompactSettings(): Promise<import('./ipc-g6.ts').CompactSettings>
  setCompactSettings(patch: import('./ipc-g6.ts').CompactSettingsPatch): Promise<void>

  // [g9] API yöntemleri
  /** Sağlayıcılar, anahtar profilleri, uç noktalar, sağlık, maliyet, yerel Ollama (bkz. ipc-g9.ts) */
  g9: import('./ipc-g9.ts').G9Api

  // [g35] Venice web hesabı (e-posta+parola+2FA girişi; API anahtarı yolundan bağımsız)
  /** Web hesabı oturumu: giriş, 2FA kodu, çıkış, durum (bkz. ipc-g35.ts) */
  g35: import('./ipc-g35.ts').G35Api
  routines: import('./ipc-routines.ts').RoutinesApi
  terminal: import('./ipc-terminal.ts').TerminalApi
  /** Git paneli: durum, dallar, push, PR (bkz. ipc-git.ts) */
  git: import('./ipc-git.ts').GitApi
  /** Notlar paneli (bkz. ipc-notes.ts) */
  notes: import('./ipc-notes.ts').NotesApi
  /** Web önizleme: dev sunucusu + tarayıcı görünümü + companion seçimleri (bkz. ipc-preview.ts) */
  preview: import('./ipc-preview.ts').PreviewApi
  /** Ajan tarayıcısı paneli: sekmeler, adres, konsol (bkz. ipc-browser.ts) */
  abrowser: import('./ipc-browser.ts').BrowserPanelApi
  /** Günlükler paneli (bkz. ipc-logs.ts) */
  logs: import('./ipc-logs.ts').LogsApi
  /** Etiketler: @ad ile eklenen metin parçacıkları (bkz. ipc-tags.ts) */
  tags: import('./ipc-tags.ts').TagsApi

  /** MCP sunucuları: ekle/güncelle/sil/etkinleştir/yeniden bağlan + durum (bkz. ipc-mcp.ts) */
  mcp: import('./ipc-mcp.ts').McpApi
  /** [g31] SSH sunucuları ve veritabanları (bkz. ipc-connections.ts) */
  connections: import('./ipc-connections.ts').ConnectionsApi

  /** Dosyadan slash komutları (bkz. ipc-commands.ts) */
  commands: import('./ipc-commands.ts').CommandsApi
  /** Görev şablonları (bkz. ipc-templates.ts) */
  templates: import('./ipc-templates.ts').TemplatesApi
  /** Yüklü ajanlar (bkz. ipc-agents.ts) */
  agents: import('./ipc-agents.ts').AgentsApi
  /** [cdx] Takımlar: rol tabanlı ajan ekipleri (bkz. ipc-teams.ts) */
  teams: import('./ipc-teams.ts').TeamsApi
  /** Chrome companion eklentisi köprüsü (bkz. ipc-chrome.ts) */
  chromeExtension: import('./ipc-chrome.ts').ChromeExtensionApi
  /** Ayarlar > Çekirdek: CLI komutlarının karşılıkları (gizlilik, undo, compat, config, pazarlar) */
  core: import('./ipc-core.ts').CoreApi

  /** Eklenti paketleri (bkz. ipc-plugins.ts) */
  plugins: import('./ipc-plugins.ts').PluginsApi

  // [g10] API yöntemleri
  /** Yarım tur kurtarma, ölü mektup kuyruğu, başlangıç öz-denetimi (bkz. ipc-g10.ts) */
  g10: import('./ipc-g10.ts').G10Api
  // [g30] Otomatik hafıza onay kuyruğu
  g30: {
    pendingList(): Promise<import('../core/auto-memory-g30.ts').PendingMemory[]>
    pendingResolve(id: string, action: 'save' | 'skip', text?: string): Promise<import('../core/auto-memory-g30.ts').PendingMemory[]>
    getAutoMemory(): Promise<boolean>
    setAutoMemory(on: boolean): Promise<void>
    onPending(listener: () => void): () => void
  }

  // [g11] API yöntemleri
  /** Oturum/proje/global hafıza katmanları, olası çakışmalar ve güven puanı ayarı */
  g11GetOverview(sessionId: string): Promise<import('../core/memory-g11.ts').MemoryOverview>
  /** Kaydı kimliğiyle siler (s/k/g önekli) */
  g11Forget(sessionId: string, id: string): Promise<void>
  /** İki kaydın ikisi de geçerli: çift bir daha çakışma sayılmaz */
  g11KeepBoth(a: string, b: string): Promise<void>
  g11SetConfidence(enabled: boolean): Promise<void>

  // [g12] API yöntemleri
  g12GetSandbox(): Promise<import('./ipc-g12.ts').SandboxStatus>
  /** Kullanıcı ayar dosyasına yazar; güncel durumu döner */
  g12SetSandbox(enabled: boolean): Promise<import('./ipc-g12.ts').SandboxStatus>
  /** Sohbet henüz başlamadıysa null */
  g12GetToolScopes(sessionId: string): Promise<import('./ipc-g12.ts').G12ToolScope[] | null>
  g12SetToolScope(sessionId: string, tool: string, level: import('./ipc-g12.ts').G12ToolLevel): Promise<import('./ipc-g12.ts').G12ToolScope[]>
  g12AnswerBulkDelete(id: string, approved: boolean): Promise<void>

  // [g13] API yöntemleri
  /** Sistem istemi katmanları, .env yöneticisi, doküman kaynakları ve bağlam dışa aktarma (bkz. ipc-g13.ts) */
  g13: import('./ipc-g13.ts').G13Api

  // [g14] API yöntemleri
  /** Gözlem panoları: izler, metrikler, maliyet, süreçler, ham istek denetçisi (bkz. ipc-g14.ts) */
  g14: import('./ipc-g14.ts').G14Api

  // [g15] API yöntemleri

  // [g16] API yöntemleri
  g16UpdateStatus(): Promise<import('./ipc-g16.ts').UpdateStatus>
  /** Güncelleme denetler; bulunursa fark tabanlı (blockmap) indirir */
  g16UpdateCheck(): Promise<import('./ipc-g16.ts').UpdateStatus>
  /** İndirilen güncellemeyi kurmak için uygulamayı yeniden başlatır */
  g16UpdateInstall(): Promise<void>
  g16CrashList(): Promise<import('./ipc-g16.ts').CrashInfo>
  /** Maskeli raporu kaydetme penceresiyle diske yazar; iptal edilirse null */
  g16CrashExport(): Promise<string | null>
  /** Kullanıcının tanımladığı uç noktaya ana süreçteki onaydan sonra gönderir; onaylanmazsa false */
  g16CrashSend(): Promise<boolean>
  g16CrashOpenFolder(): Promise<void>
  /** Tüm ayarları bildirimsel dosyaya yazar; iptal edilirse null */
  g16ConfigExport(): Promise<string | null>
  /** Dosya seçilir, özet ana süreçte onaylatılır; iptal edilirse null */
  g16ConfigImport(): Promise<import('./ipc-g16.ts').ConfigImportResult | null>

  // [g18] API yöntemleri

  // [g19] API yöntemleri

  // [g20] API yöntemleri
  /** TypeScript tiplerini siler (yalnız metin dönüşümü; kod çalışmaz) */
  g20StripTs(code: string): Promise<string>
  /** Ana süreçte yerel onaydan sonra proje Python'uyla çalıştırır; iptalde null */
  g20RunPython(code: string): Promise<import('../core/g20-run.ts').G20PythonResult | null>
  /** Çalışma dizinindeki SQLite dosyasında salt okunur sorgu (onaylı); iptalde null */
  g20RunSql(dbPath: string, sql: string): Promise<import('../core/g20-sql.ts').G20SqlResult | null>
  /** Çalışma dizinindeki .ipynb dosyasının ham metni */
  g20ReadNotebook(path: string): Promise<string>

  // [g21] API yöntemleri
  /** Belge önizlemesi için çalışma dizinindeki dosyanın ham baytları (hassas dosya reddedilir, boyut sınırlı) */
  g21ReadDoc(path: string): Promise<Uint8Array>

  // [g22] API yöntemleri
  /** Modelin yetenekleri, araç modu ve verilen eklerin nasıl gönderileceği (yerel / metne dönüştürülerek / desteklenmiyor) */
  g22Support(model: string, attachments: string[]): Promise<import('./ipc-g22.ts').G22ModelSupport>
  /** Model başına araç modu; kullanıcı ayar dosyasına yazılır ('auto' kaydı siler) */
  g22SetToolMode(model: string, mode: import('./ipc-g22.ts').ToolMode): Promise<void>

  // [g7] API yöntemleri
  /** Venice modelleri + merkezi katalogdaki Artificial Analysis puanları (yerel kopya); force: önbellekleri yenile */
  g7GetCatalog(force: boolean): Promise<import('./g7-types.ts').CatalogResponse>
  /** Araçsız tek tur test isteği; runId ile iptal edilir */
  g7RunProbe(runId: string, model: string, prompt: string): Promise<import('./g7-types.ts').ProbeResult>
  g7AbortProbe(runId: string): Promise<void>
  g7GetSettings(): Promise<import('./g7-types.ts').G7Settings>
  g7SetModelAliases(aliases: Record<string, string>): Promise<void>
  g7SetTaskModels(taskModels: import('./g7-types.ts').TaskModels): Promise<void>
  g7SetTheme(theme: import('./g7-types.ts').ThemeSetting): Promise<void>
  /** [cdx] Bildirim tercihleri (masaüstü/ses) */
  g7SetNotifications(prefs: import('./g7-types.ts').NotificationPrefs): Promise<void>
  g7SetGit(prefs: import('./g7-types.ts').GitPrefs): Promise<void>
  g7SetEditor(prefs: import('./g7-types.ts').EditorPrefs): Promise<void>
  g7SetScripts(scripts: import('./g7-types.ts').ScriptPrefs): Promise<void>
  /** [cdx] Kullanıcı tanımlı ajan profilleri (tam liste yazılır) */
  g7SetProfiles(profiles: import('./g7-types.ts').AgentProfile[]): Promise<void>
  /** [perf] Denetim günlüğü saklama süresi */
  g7SetLogs(logs: { keepDays: number }): Promise<void>
  g7GetPermissionRules(): Promise<import('./g7-types.ts').PermissionRulesInfo>
  g7AddPermissionRule(list: 'allow' | 'deny', rule: string): Promise<void>
  g7RemovePermissionRule(list: 'allow' | 'deny', rule: string): Promise<void>
  /** Açık klasörün güven durumu; klasör yoksa null */
  g7GetTrust(): Promise<import('./g7-types.ts').TrustInfo | null>
  g7SetTrust(trusted: boolean): Promise<void>
  g7GetPrivacyInfo(): Promise<import('./g7-types.ts').PrivacyInfo>
  g7OpenFolder(folder: import('./g7-types.ts').PrivacyFolder): Promise<void>
  /** mode 'trash': çöp kutusuna taşır, 'delete': kalıcı siler */
  g7ClearData(target: import('./g7-types.ts').PrivacyClearTarget, mode: 'trash' | 'delete'): Promise<void>
  g7GetUsageHistory(period: import('./g7-types.ts').UsagePeriod): Promise<import('./g7-types.ts').UsageHistory>
  /** Ana süreçten Venice API'sine erişilebiliyor mu */
  g7CheckOnline(): Promise<boolean>

  // [g17] API yöntemleri
  /** Kendine yeten HTML'i görünmez pencerede çizip PDF'e aktarır; çalışma dizinine kaydeder, iptalde null */
  g17ExportPdf(html: string, suggestedName: string): Promise<string | null>

  // [g27] geliştirme ortamı tespiti ve onaylı kurulum
  g27ListRuntimes(): Promise<import('./runtimes-g27.ts').RuntimeStatus[]>
  g27InstallRuntime(id: string): Promise<{ installed: boolean; version: string | null; opened: boolean }>
  onRuntimeProgress(listener: (tick: import('../core/runtimes-g27.ts').InstallTick & { id: string }) => void): () => void

  onEvent(listener: (event: AgentEvent) => void): () => void
}

export const CHANNELS = {
  getState: 'app:getState',
  setApiKey: 'app:setApiKey',
  openWorkspace: 'workspace:open',
  setWorkspacePath: 'workspace:setPath', // son kullanılan klasöre diyalogsuz geç
  clearWorkspace: 'workspace:clear', // klasörü kaldır (klasörsüz sohbet)
  listDir: 'fs:listDir',
  readFile: 'fs:readFile',
  listModels: 'models:list',
  setModel: 'models:set',
  setPermissionMode: 'permissions:setMode',
  setDefaultPermissionMode: 'permissions:setDefault',
  getUsage: 'usage:get',
  listSessions: 'sessions:list',
  openSession: 'sessions:open',
  liveSessions: 'sessions:live',
  deleteSession: 'sessions:delete',
  pinSession: 'sessions:pin',
  setActiveSession: 'sessions:setActive',
  getMemory: 'memory:get',
  setMemory: 'memory:set',
  send: 'agent:send',
  abort: 'agent:abort',
  approve: 'agent:approve',
  revertFile: 'fs:revert',
  getSystemPrompt: 'prompt:get',
  setSystemPrompt: 'prompt:set',
  pickFiles: 'fs:pickFiles',
  openExternal: 'shell:openExternal',

  // [g1] kanallar
  g1RenameSession: 'g1:session:rename',
  g1ArchiveSession: 'g1:session:archive',
  g1MoveSession: 'g1:session:section',
  g1LoadSessionView: 'g1:session:view',
  g1GetSessionConfig: 'g1:session:config:get',
  g1SetSessionConfig: 'g1:session:config:set',
  g1ActivateSession: 'g1:session:activate',
  g1OpenSessionWindow: 'g1:session:window',
  g1DuplicateSession: 'g1:session:duplicate', // [cdx] Çoğalt / Dallandır
  g1ExportSession: 'g1:session:export', // [cdx] Markdown dışa aktarma
  cdxGetFileChange: 'cdx:fileChange:get', // [perf] büyük dosya içeriği
  g1ListSections: 'g1:sections:list',
  g1CreateSection: 'g1:sections:create',
  g1RenameSection: 'g1:sections:rename',
  g1DeleteSection: 'g1:sections:delete',
  g1GetLastSession: 'g1:last:get',
  g1SetLastSession: 'g1:last:set',
  g1GetModelPricing: 'g1:pricing:get',
  g1SessionsChanged: 'g1:sessions:changed',

  // [g2] kanallar
  g2Send: 'g2:send',
  g2QueueRemove: 'g2:queue:remove',
  g2QueueClear: 'g2:queue:clear',
  g2QueueFlush: 'g2:queue:flush',
  g2SearchFiles: 'g2:files:search',
  g2StatFiles: 'g2:files:stat',
  g2ResolveDropped: 'g2:files:resolveDropped',
  g2ModelCaps: 'g2:models:caps',
  g2EditMessage: 'g2:history:edit',
  g2Regenerate: 'g2:history:regenerate',

  // [g3] kanallar
  g3Answer: 'g3:interact:answer',
  g3GetOutputStyle: 'g3:outputStyle:get',
  g3SetOutputStyle: 'g3:outputStyle:set',
  g3LinkPreview: 'g3:link:preview',
  g3SaveCode: 'g3:code:save',
  g3RunCode: 'g3:code:run',

  // [g4] kanallar

  // [g5] kanallar

  // [g6] kanallar
  compactSession: 'g6:context:compact',
  editCompactionSummary: 'g6:context:editSummary',
  initProject: 'g6:project:init',
  getProjectMap: 'g6:map:get',
  buildProjectMap: 'g6:map:build',
  cancelProjectMap: 'g6:map:cancel',
  getContextInfo: 'g6:context:get',
  getCompactSettings: 'g6:context:settings',
  setCompactSettings: 'g6:context:setSettings',

  // [g9] kanallar
  g9GetProviders: 'g9:providers:get',
  g9AddKey: 'g9:keys:add',
  g9RemoveKey: 'g9:keys:remove',
  g9SetActiveKey: 'g9:keys:active',
  g9SetKeyBackup: 'g9:keys:backup',
  g9TestKey: 'g9:keys:test',
  g9SetEndpoint: 'g9:endpoint:set',
  g9SaveSettings: 'g9:settings:save',
  g9Health: 'g9:health',
  g9CostInfo: 'g9:cost:info',
  g9ClearResponseCache: 'g9:cache:clear',
  g9OllamaList: 'g9:ollama:list',
  g9OllamaPull: 'g9:ollama:pull',
  g9OllamaCancelPull: 'g9:ollama:cancel',
  g9OllamaUnload: 'g9:ollama:unload',

  // MCP sunucuları (ayarlar)
  mcpList: 'mcp:list',
  commandsList: 'commands:list', // dosyadan slash komutları
  agentsList: 'agents:list', // yüklü ajanlar
  chromeInfo: 'chrome:info', // Chrome köprüsü durumu
  chromeOpenFolder: 'chrome:openFolder', // eklenti klasörünü aç
  pluginsList: 'plugins:list',
  pluginsSetEnabled: 'plugins:enabled',
  pluginsOpenFolder: 'plugins:openFolder',
  pluginsCatalog: 'plugins:catalog',
  pluginsInstall: 'plugins:install',
  pluginsUpdate: 'plugins:update',
  pluginsUninstall: 'plugins:uninstall',
  pluginsCheckUpdates: 'plugins:checkUpdates',
  pluginsAddMcp: 'plugins:addMcp',
  pluginsStoreSearch: 'plugins:storeSearch', // GitHub global mağaza arama
  pluginsInstallRepo: 'plugins:installRepo', // mağaza/elle: https depo adresinden kurulum
  pluginsInstallProgress: 'plugins:installProgress', // ana süreç -> arayüz: git clone yüzde ilerlemesi
  mcpSave: 'mcp:save',
  mcpRemove: 'mcp:remove',
  mcpSetEnabled: 'mcp:enabled',
  mcpReconnect: 'mcp:reconnect',
  // [g31] uzak bağlantılar
  connectionsList: 'connections:list',
  connectionsSaveSsh: 'connections:save-ssh',
  connectionsSaveDb: 'connections:save-db',
  connectionsRemove: 'connections:remove',
  connectionsTestSsh: 'connections:test-ssh',
  connectionsTrustHostKey: 'connections:trust-host-key',
  connectionsTestDb: 'connections:test-db',

  // [g10] kanallar
  g10ListRecoverable: 'g10:recovery:list',
  g10RecoveryInfo: 'g10:recovery:info',
  g10Restore: 'g10:recovery:restore',
  g10Dismiss: 'g10:recovery:dismiss',
  g10ListDeadLetters: 'g10:dlq:list',
  g10RemoveDeadLetter: 'g10:dlq:remove',
  g10RetryDeadLetter: 'g10:dlq:retry',
  g10SelfCheck: 'g10:selfcheck:run',
  g10InstallGit: 'g10:selfcheck:installGit', // eksik git'i otomatik kur (winget)

  // [g30] Otomatik hafıza: onay kuyruğu + açma/kapama
  g30MemoryPending: 'g30:memory:pending', // ana süreç -> arayüz: yeni aday geldi
  g30PendingList: 'g30:memory:list',
  g30PendingResolve: 'g30:memory:resolve', // kaydet/atla (+ düzenlenmiş metin)
  g30GetAutoMemory: 'g30:memory:getEnabled',
  g30SetAutoMemory: 'g30:memory:setEnabled',

  // [g11] kanallar
  g11GetOverview: 'g11:memory:overview',
  g11Forget: 'g11:memory:forget',
  g11KeepBoth: 'g11:memory:keepBoth',
  g11SetConfidence: 'g11:settings:confidence',

  // [g12] kanallar
  g12GetSandbox: 'g12:sandbox:get',
  g12SetSandbox: 'g12:sandbox:set',
  g12GetToolScopes: 'g12:tools:get',
  g12SetToolScope: 'g12:tools:set',
  g12AnswerBulkDelete: 'g12:bulkDelete:answer',

  // [g13] kanallar
  g13GetLayers: 'g13:prompt:get',
  g13SetProjectPrompt: 'g13:prompt:setProject',
  g13SetSessionPrompt: 'g13:prompt:setSession',
  g13EnvFiles: 'g13:env:files',
  g13EnvKeys: 'g13:env:keys',
  g13EnvReveal: 'g13:env:reveal',
  g13EnvSet: 'g13:env:set',
  g13EnvRemove: 'g13:env:remove',
  g13EnvTemplateReport: 'g13:env:template-report',
  g13EnvCreateTemplate: 'g13:env:create-template',
  g13GetDocSources: 'g13:docs:get',
  g13SetDocSources: 'g13:docs:set',
  g13BuildExport: 'g13:export:build',
  g13SaveExport: 'g13:export:save',

  // [g14] kanallar
  g14Overview: 'g14:overview',
  g14Cost: 'g14:cost',
  g14Raw: 'g14:raw',
  g14Processes: 'g14:processes',
  g14Probe: 'g14:probe',
  g14Status: 'g14:status',
  g14GetSettings: 'g14:settings:get',
  g14SetSettings: 'g14:settings:set',
  g14Rate: 'g14:rate',
  g14GetTags: 'g14:tags:get',
  g14SetTags: 'g14:tags:set',
  g14Alert: 'g14:alert',

  // [g15] kanallar

  // [g16] kanallar
  g16UpdateStatus: 'g16:update:status',
  g16UpdateCheck: 'g16:update:check',
  g16UpdateInstall: 'g16:update:install',
  g16CrashList: 'g16:crash:list',
  g16CrashExport: 'g16:crash:export',
  g16CrashSend: 'g16:crash:send',
  g16CrashOpenFolder: 'g16:crash:open',
  g16ConfigExport: 'g16:config:export',
  g16ConfigImport: 'g16:config:import',

  // [g18] kanallar

  // [g19] kanallar

  // [g20] kanallar
  g20StripTs: 'g20:ts:strip',
  g20RunPython: 'g20:python:run',
  g20RunSql: 'g20:sql:run',
  g20ReadNotebook: 'g20:notebook:read',

  // [g21] kanallar
  g21ReadDoc: 'g21:doc:read',

  // [g22] kanallar
  g22Support: 'g22:model:support',
  g22SetToolMode: 'g22:toolMode:set',

  // [g7] kanallar
  g7GetCatalog: 'g7:catalog:get',
  g7RunProbe: 'g7:probe:run',
  g7AbortProbe: 'g7:probe:abort',
  g7GetSettings: 'g7:settings:get',
  g7SetModelAliases: 'g7:settings:setAliases',
  g7SetTaskModels: 'g7:settings:setTaskModels',
  g7SetTheme: 'g7:settings:setTheme',
  g7SetNotifications: 'g7:settings:setNotifications',
  g7SetGit: 'g7:settings:setGit',
  g7SetEditor: 'g7:settings:setEditor',
  g7SetScripts: 'g7:settings:setScripts',
  g7SetProfiles: 'g7:settings:setProfiles',
  g7SetLogs: 'g7:settings:setLogs',
  g7GetPermissionRules: 'g7:perm:get',
  g7AddPermissionRule: 'g7:perm:add',
  g7RemovePermissionRule: 'g7:perm:remove',
  g7GetTrust: 'g7:trust:get',
  g7SetTrust: 'g7:trust:set',
  g7GetPrivacyInfo: 'g7:privacy:info',
  g7OpenFolder: 'g7:privacy:open',
  g7ClearData: 'g7:privacy:clear',
  g7GetUsageHistory: 'g7:usage:history',
  g7CheckOnline: 'g7:net:check',

  // [g17] kanallar
  g17ExportPdf: 'g17:export:pdf',

  // [g27] kanallar
  g27ListRuntimes: 'g27:runtimes:list',
  g27InstallRuntime: 'g27:runtimes:install',
  g27RuntimeProgress: 'g27:runtimes:progress',

  // [g35] kanallar (Venice web hesabı)
  g35Status: 'g35:status',
  g35Login: 'g35:login',
  g35SubmitCode: 'g35:code',
  g35Logout: 'g35:logout',
  g35Restore: 'g35:restore',

  event: 'agent:event',
} as const
