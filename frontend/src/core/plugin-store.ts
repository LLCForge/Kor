// Global eklenti mağazası: GitHub arama API'siyle (anahtarsız, yalnızca GET) kurulabilir eklenti depolarını keşfeder.
// Dönen her şey GÜVENİLMEZ veridir: zod ile doğrulanır, yalnızca https github.com klon adresleri kabul edilir, klon
// klasörü adı NAME_RE'ye indirgenir. Kurulum anında (plugin-install) adres yeniden doğrulanır ve klon kod çalıştırmaz.
import { z } from 'zod'

const GITHUB_SEARCH = 'https://api.github.com/search/repositories'
export const DEFAULT_STORE_QUERY = 'topic:claude-code-plugin'
const TIMEOUT_MS = 15_000
const PER_PAGE = 30

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

const RepoSchema = z.object({
  full_name: z.string().min(1),
  clone_url: z.string().url(),
  description: z.string().nullish(),
  stargazers_count: z.number().int().nonnegative().optional(),
  topics: z.array(z.string()).optional(),
  owner: z.object({ login: z.string() }).optional(),
})
const SearchSchema = z.object({ items: z.array(z.unknown()) })

/** Depo adresinden NAME_RE'ye (^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$) uyan klon klasörü adı türetir; türetilemezse ''. */
export function deriveInstallName(repoUrl: string): string {
  let u: URL
  try {
    u = new URL(repoUrl)
  } catch {
    return ''
  }
  const last = u.pathname.split('/').filter(Boolean).at(-1) ?? ''
  const safe = last
    .replace(/\.git$/i, '')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^[-_]+|[-_]+$/g, '')
    .slice(0, 64)
  return /^[a-z0-9]/.test(safe) ? safe : ''
}

function toStorePlugin(raw: unknown): StorePlugin | null {
  const p = RepoSchema.safeParse(raw)
  if (!p.success) return null
  const r = p.data
  let host: string
  try {
    host = new URL(r.clone_url).host
  } catch {
    return null
  }
  if (host !== 'github.com') return null // yalnızca github.com https klon adresleri
  const name = deriveInstallName(r.clone_url)
  if (!name) return null
  return {
    name,
    fullName: r.full_name,
    description: (r.description ?? '').trim(),
    author: r.owner?.login ?? r.full_name.split('/')[0] ?? '',
    repoUrl: r.clone_url,
    stars: r.stargazers_count ?? 0,
    topics: r.topics ?? [],
  }
}

/** GitHub arama yanıtını güvenli StorePlugin listesine indirger (geçersizleri atar, adrese göre tekilleştirir). */
export function mapStoreResults(data: unknown): StorePlugin[] {
  const parsed = SearchSchema.safeParse(data)
  if (!parsed.success) return []
  const out: StorePlugin[] = []
  const seen = new Set<string>()
  for (const item of parsed.data.items) {
    const sp = toStorePlugin(item)
    if (sp && !seen.has(sp.repoUrl)) {
      seen.add(sp.repoUrl)
      out.push(sp)
    }
  }
  return out
}

/** Mağazada arama (boş sorgu => varsayılan topic). fetchImpl test için enjekte edilebilir. */
export async function searchPluginStore(query: string, fetchImpl: typeof fetch = fetch): Promise<StorePlugin[]> {
  const q = query.trim() || DEFAULT_STORE_QUERY
  const url = `${GITHUB_SEARCH}?q=${encodeURIComponent(q)}&sort=stars&order=desc&per_page=${PER_PAGE}`
  const res = await fetchImpl(url, {
    headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'Kor-PluginStore' },
    signal: AbortSignal.timeout(TIMEOUT_MS),
  })
  if (res.status === 403 || res.status === 429) throw new Error('GitHub arama hız sınırı aşıldı; birkaç dakika sonra tekrar deneyin.')
  if (!res.ok) throw new Error(`Mağaza alınamadı (HTTP ${res.status}).`)
  return mapStoreResults(await res.json())
}
