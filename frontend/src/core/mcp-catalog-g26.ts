// [g26] Yerel MCP kataloğu ve PKCE. Uzak katalog yok; istemci sırrı burada durmaz.
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'

export interface McpCatalogEntry {
  id: string
  title: string
  transport: 'stdio' | 'http'
  description: string
}

/** Tanım listesi. Süreç başlatmaz, paket kurmaz. */
export const MCP_CATALOG: readonly McpCatalogEntry[] = [
  {
    id: 'filesystem',
    title: 'Dosya sunucusu',
    transport: 'stdio',
    description: 'Çalışma dizinini MCP ile açmak için kullanıcı kendi komutunu ayara yazar. Bu kayıt kurulum yapmaz.',
  },
  {
    id: 'fetch',
    title: 'HTTP okuma',
    transport: 'stdio',
    description: 'Salt okunur HTTP aracı. Ağ çağrısı ancak kullanıcı sunucuyu ayrıca etkinleştirince olur.',
  },
]

const tokenSchema = z
  .object({
    accessToken: z.string().min(1).max(8000).optional(),
    refreshToken: z.string().max(8000).optional(),
    expiresAt: z.number().optional(),
    verifier: z.string().min(1).max(200).optional(),
  })
  .refine((token) => Boolean(token.accessToken || token.verifier), 'Jeton veya doğrulama kodu gerekli')

export function pkcePair(): { verifier: string; challenge: string } {
  const verifier = randomBytes(32).toString('base64url')
  const challenge = createHash('sha256').update(verifier).digest('base64url')
  return { verifier, challenge }
}

export function authorizeUrl(input: { endpoint: string; clientId: string; redirectUri: string; challenge: string; state?: string; scope?: string }): string {
  const url = new URL(input.endpoint)
  if (url.protocol !== 'https:') throw new Error('Yetkilendirme adresi https olmalı')
  url.searchParams.set('response_type', 'code')
  url.searchParams.set('client_id', input.clientId)
  url.searchParams.set('redirect_uri', input.redirectUri)
  url.searchParams.set('code_challenge', input.challenge)
  url.searchParams.set('code_challenge_method', 'S256')
  url.searchParams.set('state', input.state ?? randomUUID())
  if (input.scope) url.searchParams.set('scope', input.scope)
  if (url.searchParams.has('client_secret')) throw new Error('Adreste istemci sırrı olamaz')
  return url.toString()
}

function tokenFile(dir: string, serverId: string): string {
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,60}$/.test(serverId)) throw new Error('Sunucu kimliği geçersiz')
  return path.join(dir, 'mcp-oauth', `${serverId}.json`)
}

export async function saveOauthToken(dir: string, serverId: string, token: z.infer<typeof tokenSchema>): Promise<void> {
  const file = tokenFile(dir, serverId)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, JSON.stringify(tokenSchema.parse(token)), { mode: 0o600 })
}

export async function readOauthToken(dir: string, serverId: string): Promise<z.infer<typeof tokenSchema> | null> {
  try {
    return tokenSchema.parse(JSON.parse(await readFile(tokenFile(dir, serverId), 'utf8')))
  } catch {
    return null
  }
}
