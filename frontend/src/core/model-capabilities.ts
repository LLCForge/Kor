// [g22] Model yetenekleri: araç çağrısı, görsel/PDF/ses girişi ve bağlam boyu için tek arayüz.
// Şimdilik Venice model_spec verisinden beslenir; g9 (çoklu sağlayıcı) birleştirmede setModelCapabilities ile doldurur.
import { z } from 'zod'
import { modelCatalogEntry } from './context.ts'

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

const known = new Map<string, ModelCapabilities>()

export function setModelCapabilities(modelId: string, caps: ModelCapabilities): void {
  known.set(modelId, caps)
}

const veniceModelSchema = z.object({
  id: z.string().min(1).max(200),
  model_spec: z
    .object({
      availableContextTokens: z.number().int().positive().optional(),
      capabilities: z
        .object({
          supportsFunctionCalling: z.boolean().optional(),
          supportsVision: z.boolean().optional(),
          supportsAudioInput: z.boolean().optional(),
          supportsResponseSchema: z.boolean().optional(),
        })
        .optional(),
    })
    .optional(),
})

/**
 * Venice /models?type=text yanıtını işler; bozuk kayıt atlanır. Dönen değer işlenen model sayısı.
 * PDF false: Venice dosya parçasını sunucuda metne çevirir (model PDF'yi yerel okumaz); yerel dönüştürme eşdeğer ve sınırları bizde.
 */
export function ingestVeniceCapabilities(json: unknown): number {
  const { data } = z.object({ data: z.array(z.unknown()).max(2000) }).parse(json)
  let count = 0
  for (const raw of data) {
    const m = veniceModelSchema.safeParse(raw)
    if (!m.success) continue
    const c = m.data.model_spec?.capabilities
    known.set(m.data.id, {
      tools: c?.supportsFunctionCalling === true,
      vision: c?.supportsVision === true,
      pdf: false,
      audio: c?.supportsAudioInput === true,
      contextTokens: m.data.model_spec?.availableContextTokens ?? null,
      responseSchema: c?.supportsResponseSchema === true,
    })
    count++
  }
  return count
}

/** Bilinmeyen model için temkinli varsayılan: araçlar denenir, diğer girişler kapalı (dönüştürülür veya reddedilir). */
export function getModelCapabilities(modelId: string): ModelCapabilities {
  const caps = known.get(modelId)
  if (caps) return caps
  // g6'nın bağlam kataloğu CLI'da da yüklenir; en azından araç desteği ve bağlam boyu oradan bilinir.
  const entry = modelCatalogEntry(modelId)
  return { tools: entry ? entry.supportsTools : 'unknown', vision: false, pdf: false, audio: false, contextTokens: entry?.contextTokens ?? null }
}
