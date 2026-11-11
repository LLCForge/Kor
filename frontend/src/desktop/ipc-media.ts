// Medya üretimi (§20, ADR 0060) köprü tipleri: `media.catalog|plan|generate|gallery|item|export|pending|
// collect|pasteRef|importRef` ve `paramCard` sohbet olayı. Rust: src-tauri/src/ns/media.rs,
// kor_providers::media_catalog, kor_agent::gallery. Yalnızca tip.

export type Capability = 'image' | 'edit' | 'upscale' | 'remove-bg' | 'video' | 'image-to-video' | 'tts' | 'asr' | 'audio' | 'music'

export type ParamKind = 'string' | 'integer' | 'number' | 'boolean' | 'enum' | 'file' | 'array' | 'object'

export type JsonValue = string | number | boolean | null | JsonValue[] | { [k: string]: JsonValue }

export interface CatalogParam {
  name: string
  kind: ParamKind
  role: string
  required: boolean
  options?: JsonValue[]
  min?: number
  max?: number
  default?: JsonValue
  description?: string
}

export interface CatalogPrice {
  amount: number
  currency: string
  unit: string
}

export interface CatalogModel {
  provider: string
  id: string
  name: string
  capabilities: Capability[]
  params: CatalogParam[]
  price: CatalogPrice | null
  source: string
  acceptsReference: boolean
}

export interface MediaCatalog {
  fetched: number
  providers: string[]
  errors: string[]
  models: CatalogModel[]
}

export interface PlanQuestion {
  name: string
  role: string
  kind: ParamKind
  description: string
  options: JsonValue[]
  min: number | null
  max: number | null
  default: JsonValue | null
}

export interface Unsupported {
  name: string
  asked: JsonValue
  nearest: JsonValue | null
  allowed: string
}

export interface Estimate {
  amount: number
  currency: string
  estimated: boolean
}

export interface MediaPlan {
  model: CatalogModel
  plan: {
    input: Record<string, JsonValue>
    questions: PlanQuestion[]
    unsupported: Unsupported[]
    estimate: Estimate | null
  }
  estimateText: string | null
  alternatives: { name: string; models: { model: CatalogModel; estimate: Estimate | null }[] }[]
  eta: number | null
  confirm: boolean
  confirmAbove: number
}

export interface GalleryItem {
  id: string
  file: string
  kind: 'image' | 'video' | 'audio' | 'other'
  capability: string | null
  provider: string
  model: string
  prompt: string
  params: JsonValue
  seed: number | null
  cost: number | null
  currency: string | null
  estimated: boolean
  secs: number
  created: number
  inputs: string[]
  parent: string | null
  action: string | null
  /** Mutlak yol (yalnızca `media.gallery`). */
  path?: string
}

export interface GalleryDetail {
  item: GalleryItem
  path: string
  chain: { item: GalleryItem; path: string }[]
  children: GalleryItem[]
}

export interface PendingJob {
  id: string
  provider: string
  model: string
  prompt: string
  capability: string | null
  started: number
}

export interface GenerateOptions {
  name?: string
  saveAs?: string
  action?: string
  derivedFrom?: string
}

export interface GenerateResult {
  jobId: string
  paths: string[]
  items: GalleryItem[]
  report: string
}

/** Ajanın sorduğu parametre kartı (`paramCard` olayı; kor_tools::ParamCard). */
export interface ParamField {
  name: string
  description: string
  options: JsonValue[]
  default: JsonValue | null
  min: number | null
  max: number | null
}

export interface ParamCard {
  title: string
  fields: ParamField[]
  notes: string[]
  cost: string | null
  eta: string | null
  confirm: boolean
  target: string | null
  fixed: Record<string, JsonValue>
}

export interface ParamCardEvent {
  type: 'paramCard'
  id: string
  card: ParamCard
}

export interface MediaGenApi {
  catalog(capability?: string, refresh?: boolean): Promise<MediaCatalog>
  plan(capability: string, model: string, prompt: string, params?: Record<string, JsonValue>): Promise<MediaPlan>
  generate(capability: string, model: string, prompt: string, params: Record<string, JsonValue>, reference?: string, options?: GenerateOptions): Promise<GenerateResult>
  gallery(): Promise<GalleryItem[]>
  item(id: string): Promise<GalleryDetail>
  export(id: string, dest: string): Promise<{ path: string }>
  pending(): Promise<PendingJob[]>
  collect(jobId?: string): Promise<{ collected: { job: string; paths: string[] }[] }>
  pasteRef(): Promise<{ id: string; path: string }>
  importRef(path: string): Promise<{ id: string; path: string }>
  onDone(listener: (p: { jobId: string; paths: string[] }) => void): () => void
}
