// Grup 9 ayarları: kullanıcı ayar dosyasındaki "g9" nesnesi. Yalnızca kullanıcı dosyasından okunur; depoyla gelen
// bir proje dosyası anahtarları başka adrese yönlendiremez, maliyet tavanını kaldıramaz veya önbellek açamaz.
import { z } from 'zod'
import { isLoopbackHost, PROVIDER_IDS } from './providers.ts'
import { loadUserSettingsFile, updateUserSettings } from './settings.ts'

/** Kimlik ve taşıma başlıkları kullanıcı başlıklarıyla ezilemez (anahtar sızdırma / istek kaçakçılığı). */
export const FORBIDDEN_HEADER =
  /^(authorization|proxy-authorization|x-api-key|api-key|cookie|set-cookie|host|content-length|content-type|connection|transfer-encoding|te|upgrade|anthropic-version)$/i

const headerName = z
  .string()
  .regex(/^[A-Za-z0-9-]{1,64}$/, 'Başlık adı yalnızca harf, rakam ve - içerebilir')
  .refine((n) => !FORBIDDEN_HEADER.test(n), 'Bu başlık ayarlanamaz (kimlik veya taşıma başlığı)')

const baseUrlSchema = z
  .string()
  .max(300)
  .url('Geçerli bir URL girin')
  .refine((u) => {
    const x = new URL(u)
    return x.protocol === 'https:' || (x.protocol === 'http:' && isLoopbackHost(x.hostname))
  }, 'Temel URL https olmalı; http yalnızca bu makine (localhost) için kabul edilir')
  .refine((u) => {
    const x = new URL(u)
    return !x.username && !x.password && !x.search && !x.hash
  }, 'Temel URL kullanıcı adı, parola, sorgu veya # içeremez')
  .transform((u) => u.replace(/\/+$/, ''))

export const endpointSchema = z
  .object({
    baseURL: baseUrlSchema.optional(),
    headers: z
      .record(headerName, z.string().max(500).regex(/^[^\r\n]*$/, 'Başlık değeri satır sonu içeremez'))
      .refine((o) => Object.keys(o).length <= 10, 'En fazla 10 başlık')
      .optional(),
    timeoutMs: z.number().int().min(1_000).max(600_000).optional(),
    /** Sağlayıcıya aynı anda en fazla bu kadar istek (yerel kuyruk) */
    maxConcurrent: z.number().int().min(1).max(32).optional(),
  })
  .strict()
export type ProviderEndpoint = z.infer<typeof endpointSchema>

const usd = z.number().nonnegative().max(10_000)
const priceEntrySchema = z
  .object({
    /** USD / 1M token */
    input: usd,
    output: usd,
    cacheInput: usd.optional(),
    /** Fiyat tablosu bağlam boyutunu da bildirebilir (Venice dışı listeler çoğunlukla vermez) */
    contextTokens: z.number().int().positive().max(20_000_000).optional(),
  })
  .strict()
export type PriceEntry = z.infer<typeof priceEntrySchema>

const presetSchema = z
  .object({
    temperature: z.number().min(0).max(2).optional(),
    top_p: z.number().min(0).max(1).optional(),
    maxTokens: z.number().int().min(1).max(1_000_000).optional(),
  })
  .strict()
export type ParamPreset = z.infer<typeof presetSchema>

const PRESET_NAME = /^[\p{L}\p{N} _.-]{1,40}$/u
const presetName = z.string().regex(PRESET_NAME, 'Ön ayar adı en fazla 40 karakter: harf, rakam, boşluk, _ . -')
const modelId = z.string().min(1).max(200)
const atMost = (max: number, what: string) => [(o: object) => Object.keys(o).length <= max, `En fazla ${max} ${what}`] as const

const responseCacheSchema = z
  .object({
    enabled: z.boolean(),
    ttlHours: z.number().int().min(1).max(720),
    /** Bu metinlerden birini içeren istemler önbelleğe alınmaz (büyük/küçük harf duyarsız) */
    exclude: z.array(z.string().min(2).max(100)).max(50),
  })
  .strict()
export type ResponseCacheSettings = z.infer<typeof responseCacheSchema>

export const g9SettingsSchema = z
  .object({
    endpoints: z.partialRecord(z.enum(PROVIDER_IDS), endpointSchema),
    ollamaEnabled: z.boolean(),
    presets: z.record(presetName, presetSchema).refine(...atMost(30, 'ön ayar')),
    modelPresets: z.record(modelId, presetName).refine(...atMost(300, 'model ataması')),
    prices: z.record(modelId, priceEntrySchema).refine(...atMost(500, 'fiyat satırı')),
    /** Bir görevin (tur + alt ajanları) harcayabileceği azami tutar, USD */
    costCapUsd: z.number().positive().max(10_000).nullable(),
    /** Başlık ve özetleme gibi yan işler, açıkça seçilmemişse en ucuz yeterli modele gider */
    lowCostMode: z.boolean(),
    responseCache: responseCacheSchema,
  })
  .partial()
  .strict()
export type G9Settings = z.infer<typeof g9SettingsSchema>

export const DEFAULT_RESPONSE_CACHE: ResponseCacheSettings = { enabled: false, ttlHours: 24, exclude: [] }

/** Kullanıcı dosyasındaki g9 nesnesi; bozuksa uyarı verilip varsayılanlara düşülür (uygulama kilitlenmesin). */
export async function loadG9Settings(file?: string): Promise<G9Settings> {
  const raw = (await loadUserSettingsFile(file)).g9
  if (raw === undefined) return {}
  const parsed = g9SettingsSchema.safeParse(raw)
  if (parsed.success) return parsed.data
  console.warn(`Sağlayıcı/maliyet ayarları (g9) geçersiz, varsayılanlar kullanılıyor:\n${z.prettifyError(parsed.error)}`)
  return {}
}

/** Diğer ayar alanları korunur; sonuç şemaya uymazsa hiçbir şey yazılmaz. */
export async function updateG9Settings(mutate: (current: G9Settings) => G9Settings, file?: string): Promise<G9Settings> {
  let result: G9Settings = {}
  await updateUserSettings((current) => {
    const existing = g9SettingsSchema.safeParse(current.g9 ?? {})
    const next = g9SettingsSchema.safeParse(mutate(existing.success ? existing.data : {}))
    if (!next.success) throw new Error(`Geçersiz ayar:\n${z.prettifyError(next.error)}`)
    result = next.data
    return { ...current, g9: next.data }
  }, file)
  return result
}
