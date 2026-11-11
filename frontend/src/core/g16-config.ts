// [g16] Yapılandırma olarak kod (199) ve kurumsal merkezi yapılandırma (198).
// Bildirimsel dosya: kullanıcı settings.json + özel sistem istemi + çıktı stili tek JSON'da dışa aktarılır ve kurulur.
// Kurumsal politika: MDM'nin makineye bıraktığı sabit yoldaki dosya; yalnızca güncelleme kaynağını belirler.
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { OUTPUT_STYLE_IDS, type OutputStyle } from './output-styles.ts'
import type { UserSettingsFile } from './settings.ts'

export const CONFIG_KIND = 'kor-config'
const MAX_PROMPT_CHARS = 100_000

/**
 * Başkasından gelen bir dosyanın yazamayacağı alanlar: izinleri genişletir, komut çalıştırır, çalışma alanını büyütür
 * veya veriyi/kurulumu dış bir sunucuya bağlar. Bunlar yalnızca kullanıcının kendi settings.json'unda düzenlenir.
 */
export const PROTECTED_SETTINGS = ['permissions.mode', 'permissions.allow', 'shell', 'additionalDirectories', 'autoVerify', 'updates', 'crashReports'] as const
const PROTECTED_TOP = new Set(['shell', 'additionalDirectories', 'autoVerify', 'updates', 'crashReports'])

export const declarativeConfigSchema = z
  .object({
    kind: z.literal(CONFIG_KIND),
    version: z.literal(1),
    exportedAt: z.string().max(64).optional(),
    /** Ayrıntılı doğrulama yazarken settings.ts şemasıyla yapılır (bilinmeyen alan reddedilir). */
    settings: z.record(z.string(), z.unknown()).optional(),
    /** null: özel istem yok (varsayılan) */
    systemPrompt: z.string().max(MAX_PROMPT_CHARS).nullable().optional(),
    outputStyle: z.enum(OUTPUT_STYLE_IDS).optional(),
  })
  .strict()

export type DeclarativeConfig = z.infer<typeof declarativeConfigSchema>

export function buildDeclarativeConfig(
  settings: UserSettingsFile,
  systemPrompt: string | null,
  outputStyle: OutputStyle,
  now: Date = new Date(),
): DeclarativeConfig {
  return { kind: CONFIG_KIND, version: 1, exportedAt: now.toISOString(), settings: { ...settings }, systemPrompt, outputStyle }
}

/** JSON metnini doğrular; hata mesajı kullanıcıya gösterilebilir (dosya içeriği yansıtılmaz). */
export function parseDeclarativeConfig(text: string): DeclarativeConfig {
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error('Yapılandırma dosyası geçerli JSON değil')
  }
  const parsed = declarativeConfigSchema.safeParse(json)
  if (!parsed.success) throw new Error(`Yapılandırma dosyası geçersiz:\n${z.prettifyError(parsed.error)}`)
  return parsed.data
}

export interface ImportSummary {
  /** Yazılacak ayar anahtarları (ör. model, theme, permissions.deny) */
  applied: string[]
  /** Dosyada olup güvenlik nedeniyle yok sayılanlar */
  skipped: string[]
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v)

export function summarizeImport(imported: Record<string, unknown>): ImportSummary {
  const applied: string[] = []
  const skipped: string[] = []
  for (const [key, value] of Object.entries(imported)) {
    if (PROTECTED_TOP.has(key)) skipped.push(key)
    else if (key === 'permissions' && isRecord(value)) {
      for (const sub of Object.keys(value)) (sub === 'deny' ? applied : skipped).push(`permissions.${sub}`)
    } else applied.push(key)
  }
  return { applied, skipped }
}

/**
 * Dosyadaki anahtarlar mevcut ayarın üzerine yazılır, dosyada olmayanlar korunur. Korunan alanlar her zaman mevcut
 * dosyadan gelir; deny kuralları yalnızca eklenir (içe aktarma bir yasağı kaldıramaz). Sonuç yazılmadan önce
 * settings.ts şemasıyla doğrulanır (updateUserSettings).
 */
export function mergeImportedSettings(current: UserSettingsFile, imported: Record<string, unknown>): Record<string, unknown> {
  const next: Record<string, unknown> = { ...current }
  for (const [key, value] of Object.entries(imported)) {
    if (PROTECTED_TOP.has(key) || key === 'permissions') continue
    next[key] = value
  }
  const importedDeny = isRecord(imported.permissions) && Array.isArray(imported.permissions.deny) ? imported.permissions.deny : []
  if (importedDeny.length > 0) {
    const deny = [...new Set([...(current.permissions?.deny ?? []), ...importedDeny])]
    next.permissions = { ...current.permissions, deny }
  }
  return next
}

// ── Kurumsal politika (198) ─────────────────────────────────────────────────────────────────

const httpsUrl = z.url({ protocol: /^https$/ }).max(2048)

export const enterprisePolicySchema = z
  .object({
    updates: z.object({ url: httpsUrl, autoCheck: z.boolean(), disabled: z.boolean() }).partial().strict(),
  })
  .partial()
  .strict()

export type EnterprisePolicy = z.infer<typeof enterprisePolicySchema>

/**
 * MDM'nin yapılandırma dosyasını bırakacağı makine geneli yol. Windows'ta ProgramData altında klasör oluşturmak
 * varsayılan ACL ile standart kullanıcıya da açıktır; politika bu yüzden kullanıcının kendi settings.json'unun
 * verebileceğinden fazlasını vermez (yalnızca https güncelleme kaynağı veya kapatma).
 */
export function enterprisePolicyFile(platform: NodeJS.Platform = process.platform, env: NodeJS.ProcessEnv = process.env): string {
  if (platform === 'win32') return path.win32.join(env.ProgramData || env.PROGRAMDATA || 'C:\\ProgramData', 'Kor', 'policy.json')
  if (platform === 'darwin') return '/Library/Application Support/Kor/policy.json'
  return '/etc/kor/policy.json'
}

/** Dosya yoksa boş politika; bozuksa hata döner ve çağıran güncellemeyi kapalı tutar (fail closed). */
export async function loadEnterprisePolicy(file: string = enterprisePolicyFile()): Promise<{ policy: EnterprisePolicy; error: string | null }> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return { policy: {}, error: null }
    return { policy: {}, error: `Kurumsal yapılandırma okunamadı: ${(err as Error).message}` }
  }
  try {
    const parsed = enterprisePolicySchema.safeParse(JSON.parse(text))
    if (parsed.success) return { policy: parsed.data, error: null }
    return { policy: {}, error: `Kurumsal yapılandırma geçersiz (${file}):\n${z.prettifyError(parsed.error)}` }
  } catch {
    return { policy: {}, error: `Kurumsal yapılandırma geçerli JSON değil (${file})` }
  }
}

export interface UpdateSource {
  url: string | null
  source: 'kurumsal' | 'kullanıcı' | null
  autoCheck: boolean
  /** Güncelleme neden kapalı (url varken bile) */
  blockedReason: string | null
}

/** Kurumsal politika kullanıcı ayarını ezer; ikisi de yoksa güncelleme kapalıdır (varsayılan). */
export function resolveUpdateSource(
  user: { url?: string; autoCheck?: boolean } | undefined,
  policy: EnterprisePolicy,
  policyError: string | null,
): UpdateSource {
  if (policyError) return { url: null, source: null, autoCheck: false, blockedReason: policyError }
  if (policy.updates?.disabled) return { url: null, source: 'kurumsal', autoCheck: false, blockedReason: 'Güncellemeler kurumsal yapılandırmayla kapatıldı' }
  if (policy.updates?.url) return { url: policy.updates.url, source: 'kurumsal', autoCheck: policy.updates.autoCheck ?? false, blockedReason: null }
  if (user?.url) return { url: user.url, source: 'kullanıcı', autoCheck: user.autoCheck ?? false, blockedReason: null }
  return { url: null, source: null, autoCheck: false, blockedReason: 'Güncelleme sunucusu yapılandırılmadı' }
}
