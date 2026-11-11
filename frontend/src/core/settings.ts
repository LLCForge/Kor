import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { projectDotDir, userConfigDir } from './layout-g28.ts' // [g28]
import { z } from 'zod'
import { PERMISSION_MODES, permissionRuleSchema, type PermissionConfig, type PermissionMode } from './permissions.ts'

export const DEFAULT_MODEL = 'deepseek-v4-1-flash-uncensored'
/** [g6] Bağlam özetleme modeli: Venice listesinde 1M bağlamlı, araç destekli ve en ucuzlardan (2026-10 itibarıyla doğrulandı). */
export const DEFAULT_COMPACT_MODEL = 'deepseek-v4-flash'
export const VENICE_BASE_URL = 'https://api.venice.ai/api/v1'
/** VENICE_AGENT_HOME: testlerin ve taşınabilir kurulumların gerçek kullanıcı klasörüne dokunmaması için. */
/** KOR_HOME test ve taşınabilir kurulum içindir. Varsayılan ~/.kor. Eski ~/.kor doluysa bir kez kopyalanır; kopya tutmazsa eski klasör silinmez. */
export const USER_CONFIG_DIR = userConfigDir()

const fileSchema = z
  .object({
    model: z.string().min(1).max(200),
    maxTurns: z.number().int().min(0).max(100000), // 0 = sınırsız

    // [g1] ayar dosyası alanları (zod)

    // [g2] ayar dosyası alanları (zod)

    // [g3] ayar dosyası alanları (zod)

    // [g4] ayar dosyası alanları (zod)

    // [g5] ayar dosyası alanları (zod)
    // Yalnızca kullanıcı dosyasından okunur (settings-g5.ts); proje dosyalarındaki değerler yok sayılır.
    shell: z.enum(['cmd', 'powershell', 'pwsh', 'git-bash']),
    additionalDirectories: z.array(z.string().min(1).max(4096)).max(50),
    autoVerify: z.boolean(),

    // [g6] ayar dosyası alanları (zod)
    compactModel: z.string().min(1).max(200),
    /** Otomatik sıkıştırma eşiği tavanı (token). null: varsayılan 48k. */
    compactAtTokens: z.number().int().min(2_000).max(500_000).nullable().optional(),
    /** Otomatik sıkıştırma oranı (bağlam sınırına göre, 0.3-0.95). null: varsayılan 0.7. */
    compactRatio: z.number().min(0.3).max(0.95).nullable().optional(),
    projectMapAutoCreate: z.enum(['ask', 'always', 'never']),

    // [g9] ayar dosyası alanları (zod)
    /** Sağlayıcı, maliyet ve önbellek ayarları; ayrıntılı şema settings-g9.ts (yalnızca kullanıcı dosyasından okunur) */
    g9: z.record(z.string().max(64), z.unknown()),

    // [g10] ayar dosyası alanları (zod)
    // Yalnızca kullanıcı dosyasından okunur (agent-g10.ts loadG10Config): yedek model maliyet doğurur.
    g10: z.object({ toolTimeoutSec: z.number().int().min(10).max(3600), firstTokenTimeoutSec: z.number().int().min(10).max(600), fallbackModel: z.string().min(1).max(200) }).partial().strict(),

    // [g11] ayar dosyası alanları (zod)
    /** Yanıt sonunda güven puanı (varsayılan açık); agent-g11.ts yalnızca kullanıcı dosyasından okur. */
    confidenceScore: z.boolean(),

    // [g12] ayar dosyası alanları (zod)
    // Çekirdek düzeyi sandbox; yalnızca kullanıcı dosyasından okunur (sandbox-g12.ts), proje dosyalarındaki değer yok sayılır.
    sandbox: z.boolean(),

    // [g13] ayar dosyası alanları (zod)
    // Onaylı doküman kaynakları (URL önekleri): docs_fetch yalnızca bunlardan okur. Yalnızca kullanıcı dosyasından okunur (prompt-g13.ts).
    docSources: z.array(z.string().max(500).regex(/^https?:\/\/[^\s/?#]+[^\s?#]*$/, 'Kaynak http(s) adresi olmalı (sorgu ve # olmadan)')).max(50),

    // [g14] ayar dosyası alanları (zod)
    // Gözlem: yalnızca kullanıcı dosyasından okunur (handlers/g14.ts); iz dışa aktarma ve yerel uç noktalar varsayılan kapalı.
    observability: z
      .object({
        traceExport: z.boolean(),
        otlpEndpoint: z.string().max(300),
        prometheus: z.boolean(),
        prometheusPort: z.number().int().min(1024).max(65535),
        slowToolMs: z.number().int().min(500).max(3_600_000),
        turnBudgetMs: z.number().int().min(5_000).max(86_400_000),
        memoryBudgetMb: z.number().int().min(128).max(65_536),
        alerts: z.boolean(),
      })
      .partial()
      .strict(),

    // [g15] ayar dosyası alanları (zod)

    // [g16] ayar dosyası alanları (zod)
    // Yalnızca kullanıcı dosyasından okunur (handlers/g16.ts); içe aktarılan yapılandırma bu alanları yazamaz (g16-config.ts).
    updates: z.object({ url: z.url({ protocol: /^https$/ }).max(2048), autoCheck: z.boolean() }).partial().strict(),
    crashReports: z.object({ uploadUrl: z.url({ protocol: /^https$/ }).max(2048) }).partial().strict(),

    // [g18] ayar dosyası alanları (zod)

    // [g19] ayar dosyası alanları (zod)

    // [g20] ayar dosyası alanları (zod)

    // [g21] ayar dosyası alanları (zod)

    // [g22] ayar dosyası alanları (zod)
    // Model kimliği → araç modu. Yalnızca kullanıcı dosyasından okunur (text-tools-g22.ts).
    toolModes: z
      .record(z.string().min(1).max(200), z.enum(['auto', 'native', 'text', 'off']))
      .refine((r) => Object.keys(r).length <= 500, 'En fazla 500 model için araç modu kaydedilebilir'),

    // [g7] ayar dosyası alanları (zod)
    /** Takma ad → model kimliği, ör. { "fast": "qwen3-6-35b-a3b" } */
    modelAliases: z.record(z.string().regex(/^[a-z][a-z0-9_-]{0,31}$/, 'Takma ad küçük harfle başlamalı; harf, rakam, - ve _ (en fazla 32)'), z.string().min(1).max(200)),
    /** Görev tipine göre model (model kimliği veya takma ad); boşsa ana model */
    taskModels: z
      .object({ explore: z.string().min(1).max(200), plan: z.string().min(1).max(200), review: z.string().min(1).max(200), title: z.string().min(1).max(200), compaction: z.string().min(1).max(200) })
      .partial()
      .strict(),
    theme: z.enum(['system', 'light', 'dark']),
    // [cdx] Bildirimler (cdesktop Notifications): masaüstü bildirimi + ses; yalnızca kullanıcı dosyasından okunur.
    notifications: z.object({ desktop: z.boolean(), sound: z.boolean(), soundName: z.enum(['soft', 'double', 'bell', 'chime']) }).partial().strict(),
    // [cdx] Git tercihleri (cdesktop Settings > Git): dal öneki (kor/1a2b-gorev) ve worktree klasörü.
    git: z.object({ branchPrefix: z.string().max(40).regex(/^[A-Za-z0-9._\/-]*$/, 'Dal öneki yalnızca harf, rakam, . _ / -'), worktreeDir: z.string().max(4096) }).partial().strict(),
    // [cdx] Betikler (cdesktop Setup / Cleanup / Dev Server script): tüm katmanlardan okunur (proje dosyası tanımlayabilir;
    // çalıştırma yalnızca güvenilen klasörde ve kullanıcı bir tur başlatınca olur).
    scripts: z.object({ setup: z.string().max(2000), cleanup: z.string().max(2000), dev: z.string().max(2000) }).partial().strict(),
    // [cdx] Ajan yapılandırma profilleri (cdesktop Agent Configuration Profiles): ad + izin modu + model + akıl yürütme.
    profiles: z
      .array(
        z
          .object({
            name: z.string().trim().min(1).max(40),
            mode: z.enum(PERMISSION_MODES).optional(),
            model: z.string().min(1).max(200).optional(),
            effort: z.enum(['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max']).optional(),
          })
          .strict(),
      )
      .max(20),
    // [perf] Denetim günlüğü saklama süresi (gün); açılışta eski dosyalar silinir.
    logs: z.object({ keepDays: z.number().int().min(1).max(3650) }).partial().strict(),
    // [cdx] Editör tercihi (cdesktop Editor Type + Remote SSH)
    editor: z.object({ type: z.enum(['code', 'cursor', 'windsurf', 'zed', 'custom', 'system']), customCommand: z.string().max(500), sshHost: z.string().max(200) }).partial().strict(),
    permissions: z
      .object({
        mode: z.enum(PERMISSION_MODES),
        allow: z.array(permissionRuleSchema).max(200),
        deny: z.array(permissionRuleSchema).max(200),
        autoMemory: z.boolean(), // hafızaya kaydetmede onay sorma
      })
      .partial()
      .strict(),
  })
  .partial()
  .strict()

type SettingsFile = z.infer<typeof fileSchema>

const cliSchema = z
  .object({
    model: z.string().min(1).max(200),
    maxTurns: z.number().int().min(0).max(100000), // 0 = sınırsız
    permissionMode: z.enum(PERMISSION_MODES),
  })
  .partial()
  .strict()

export type SettingsOverrides = z.infer<typeof cliSchema>

export interface Settings {
  model: string
  maxTurns: number
  permissions: PermissionConfig

  // [g1] Settings alanları

  // [g2] Settings alanları

  // [g3] Settings alanları

  // [g4] Settings alanları

  // [g5] Settings alanları

  // [g6] Settings alanları
  /** Bağlam sıkıştırmada özeti üreten model */
  compactModel: string
  /** Otomatik sıkıştırma eşiği tavanı (token); null = 48k varsayılan */
  compactAtTokens: number | null
  /** Otomatik sıkıştırma oranı; null = 0.7 varsayılan */
  compactRatio: number | null
  /** Proje ilk açıldığında harita: sor / otomatik oluştur / asla */
  projectMapAutoCreate: 'ask' | 'always' | 'never'

  // [g9] Settings alanları

  // [g10] Settings alanları

  // [g11] Settings alanları

  // [g12] Settings alanları

  // [g13] Settings alanları

  // [g14] Settings alanları

  // [g15] Settings alanları

  // [g16] Settings alanları

  // [g18] Settings alanları

  // [g19] Settings alanları

  // [g20] Settings alanları

  // [g21] Settings alanları

  // [g22] Settings alanları

  // [g7] Settings alanları
  /** Tüm katmanlardan birleşik (sonraki katman aynı adı ezer) */
  modelAliases: Record<string, string>
  /** Ham değerler; takma adlar model-catalog.ts resolveTaskModels ile çözülür */
  taskModels: Partial<Record<'explore' | 'plan' | 'review' | 'title' | 'compaction', string>>
  /** [cdx] Kurulum (ajan başlamadan önce, oturumda bir kez), temizlik (tur bitince) ve dev sunucu betikleri */
  scripts: { setup: string; cleanup: string; dev: string }
}

export function settingsFiles(root: string | null): { user: string; project: string; local: string } {
  return {
    user: path.join(USER_CONFIG_DIR, 'settings.json'),
    project: root ? path.join(projectDotDir(root), 'settings.json') : path.join(USER_CONFIG_DIR, 'settings.json'),
    local: root ? path.join(projectDotDir(root), 'settings.local.json') : path.join(USER_CONFIG_DIR, 'settings.json'),
  }
}

async function loadSettingsFile(file: string): Promise<SettingsFile> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return {}
    throw err
  }
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`${file}: geçersiz JSON`)
  }
  const parsed = fileSchema.safeParse(json)
  if (!parsed.success) throw new Error(`${file}:\n${z.prettifyError(parsed.error)}`)
  return parsed.data
}

/** Kullanıcı ayar dosyasındaki diğer alanları koruyarak varsayılan izin modunu yazar. */
export async function saveUserPermissionMode(mode: PermissionMode): Promise<void> {
  const file = settingsFiles('.').user
  const current = await loadSettingsFile(file)
  const next: SettingsFile = { ...current, permissions: { ...current.permissions, mode } }
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(next, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 })
}

/** Yalnızca kullanıcı dosyasındaki mod; menüde "Varsayılan" rozeti için. */
export async function loadUserPermissionMode(): Promise<PermissionMode> {
  return (await loadSettingsFile(settingsFiles('.').user)).permissions?.mode ?? 'default'
}

function definedOnly<T extends object>(o: T): Partial<T> {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as Partial<T>
}

/**
 * Öncelik: CLI > ortam değişkeni > yerel proje > proje > kullanıcı > varsayılan.
 * Güvenlik: proje dosyaları depoyla birlikte gelebilir; bu yüzden izin modu ve allow kuralları
 * yalnızca kullanıcı dosyasından veya CLI'dan okunur. Proje dosyaları sadece deny ekleyebilir.
 */
/** Duvar kökü null olabilir (klasör açılmamış): proje katmanları o zaman yalnızca kullanıcı dosyasından gelir. */
export async function loadSettings(root: string | null, cliOverrides: SettingsOverrides = {}): Promise<Settings> {
  const cli = cliSchema.safeParse(definedOnly(cliOverrides))
  if (!cli.success) throw new Error(`Geçersiz komut satırı argümanı:\n${z.prettifyError(cli.error)}`)

  const files = settingsFiles(root)
  const [user, project, local] = await Promise.all([files.user, files.project, files.local].map(loadSettingsFile))
  const layers = [user, project, local]

  const last = <K extends 'model' | 'maxTurns'>(key: K) => layers.findLast((l) => l[key] !== undefined)?.[key]
  const mode: PermissionMode = cli.data.permissionMode ?? user.permissions?.mode ?? 'default'

  return {
    model: cli.data.model ?? (process.env.VENICE_MODEL || undefined) ?? last('model') ?? DEFAULT_MODEL,
    maxTurns: cli.data.maxTurns ?? last('maxTurns') ?? 0, // varsayılan: sınırsız (adım sınırı yok)
    permissions: {
      mode,
      allow: user.permissions?.allow ?? [],
      deny: layers.flatMap((l) => l.permissions?.deny ?? []),
      // Kullanıcının doğrudan isteğiyle hafıza kaydı onaysız: yalnızca kullanıcı dosyasından ve açıkça false ise kapanır.
      autoMemory: user.permissions?.autoMemory ?? true,
    },
    // [g6] Maliyet doğuran bu iki ayar yalnızca kullanıcı dosyasından: depoyla gelen proje dosyası pahalı model veya otomatik keşif açamaz.
    compactModel: user.compactModel ?? DEFAULT_COMPACT_MODEL,
    compactAtTokens: user.compactAtTokens ?? null,
    compactRatio: user.compactRatio ?? null,
    projectMapAutoCreate: user.projectMapAutoCreate ?? 'ask',
    // [g9] loadSettings dönüş alanları

    // [g10] loadSettings dönüş alanları

    // [g11] loadSettings dönüş alanları

    // [g12] loadSettings dönüş alanları

    // [g13] loadSettings dönüş alanları

    // [g14] loadSettings dönüş alanları

    // [g15] loadSettings dönüş alanları

    // [g16] loadSettings dönüş alanları

    // [g18] loadSettings dönüş alanları

    // [g19] loadSettings dönüş alanları

    // [g20] loadSettings dönüş alanları

    // [g21] loadSettings dönüş alanları

    // [g22] loadSettings dönüş alanları

    // [g7] Takma adlar yalnızca kullanıcı yazınca kullanılır: tüm katmanlardan okunabilir.
    modelAliases: Object.fromEntries(layers.flatMap((l) => Object.entries(l.modelAliases ?? {}))),
    // Görev modelleri kendiliğinden maliyet doğurur (alt ajanlar, başlık, sıkıştırma): compactModel gibi yalnızca kullanıcı dosyasından.
    taskModels: { ...(user.taskModels ?? {}) },
    // [cdx] Betikler: sonraki katman aynı anahtarı ezer (proje > kullanıcı).
    scripts: {
      setup: layers.findLast((l) => l.scripts?.setup !== undefined)?.scripts?.setup ?? '',
      cleanup: layers.findLast((l) => l.scripts?.cleanup !== undefined)?.scripts?.cleanup ?? '',
      dev: layers.findLast((l) => l.scripts?.dev !== undefined)?.scripts?.dev ?? '',
    },
  }
}

// [g9] ayar yardımcıları

// [g10] ayar yardımcıları

// [g11] ayar yardımcıları

// [g12] ayar yardımcıları

// [g13] ayar yardımcıları

// [g14] ayar yardımcıları

// [g15] ayar yardımcıları

// [g16] ayar yardımcıları

// [g18] ayar yardımcıları

// [g19] ayar yardımcıları

// [g20] ayar yardımcıları

// [g21] ayar yardımcıları

// [g22] ayar yardımcıları

// [g7] kullanıcı ayar dosyasına güvenli yazma: diğer alanlar korunur, sonuç şemaya karşı doğrulanır,
// yazma atomiktir (geçici dosya + taşıma) ve eşzamanlı yazmalar sıraya alınır.
export type UserSettingsFile = SettingsFile

let userWriteQueue: Promise<unknown> = Promise.resolve()

export async function loadUserSettingsFile(file: string = settingsFiles('.').user): Promise<UserSettingsFile> {
  return loadSettingsFile(file)
}

export async function updateUserSettings(
  mutate: (current: UserSettingsFile) => UserSettingsFile,
  file: string = settingsFiles('.').user,
): Promise<UserSettingsFile> {
  const run = async () => {
    // Bozuk dosya burada hata verir ve üzerine yazılmaz: kullanıcının elle düzeltmesi gerekir.
    const current = await loadSettingsFile(file)
    const parsed = fileSchema.safeParse(mutate(structuredClone(current)))
    if (!parsed.success) throw new Error(`Geçersiz ayar:\n${z.prettifyError(parsed.error)}`)
    await mkdir(path.dirname(file), { recursive: true })
    const tmp = `${file}.${process.pid}.tmp`
    await writeFile(tmp, `${JSON.stringify(parsed.data, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 })
    await rename(tmp, file)
    return parsed.data
  }
  const result = userWriteQueue.then(run, run)
  userWriteQueue = result.catch(() => undefined)
  return result
}

/**
 * Kural sözdizimi permissionRuleSchema ile doğrulanır; knownTools verilirse araç adı da denetlenir
 * (yanlış yazılmış bir araç adı hiçbir şeyle eşleşmeyen, sessizce etkisiz bir kural olurdu). Aynı kural iki kez eklenmez.
 */
export async function addPermissionRule(
  list: 'allow' | 'deny',
  rule: string,
  opts: { file?: string; knownTools?: readonly string[] } = {},
): Promise<UserSettingsFile> {
  const parsed = permissionRuleSchema.safeParse(rule.trim())
  if (!parsed.success) throw new Error('Kural "arac" veya "arac(desen)" biçiminde olmalı, ör. bash(npm test*)')
  const valid = parsed.data
  const tool = /^[a-z_]+/.exec(valid)?.[0] ?? ''
  if (opts.knownTools && !opts.knownTools.includes(tool)) throw new Error(`Bilinmeyen araç: ${tool}. Geçerli araçlar: ${opts.knownTools.join(', ')}`)
  return updateUserSettings((current) => {
    const rules = current.permissions?.[list] ?? []
    return rules.includes(valid) ? current : { ...current, permissions: { ...current.permissions, [list]: [...rules, valid] } }
  }, opts.file)
}

export async function removePermissionRule(list: 'allow' | 'deny', rule: string, file?: string): Promise<UserSettingsFile> {
  return updateUserSettings((current) => {
    const rules = current.permissions?.[list]
    if (!rules?.includes(rule)) return current
    return { ...current, permissions: { ...current.permissions, [list]: rules.filter((r) => r !== rule) } }
  }, file)
}
