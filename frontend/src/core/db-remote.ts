// Veritabanı bağlantısı: SALT OKUNUR sorgu ve şema keşfi (PostgreSQL, MySQL/MariaDB, SQLite). Gerekirse SSH tüneliyle
// yalnızca sunucuda açık olan veritabanına bağlanılır (ssh-remote.ts openTunnel).
// Salt okunurluk katmanları:
//   1. Başlangıç anahtar sözcüğü allowlist'i ve tek ifade (g20-sql.ts readOnlySqlProblem); "SELECT … INTO" reddedilir.
//   2. Sunucu tarafında salt okunur işlem: PostgreSQL "BEGIN TRANSACTION READ ONLY", MySQL "START TRANSACTION READ ONLY";
//      sonunda her zaman ROLLBACK. Veri değiştiren CTE'ler (WITH … DELETE) bu katmanda düşer.
//   3. Sorgu zaman aşımı ve satır sınırı. En sağlam koruma: bağlantıda salt okunur bir veritabanı kullanıcısı kullanmak.
import type { Duplex } from 'node:stream'
import { revealSecret, type DbConnection, type SshHost } from './connections.ts'
import { readOnlySqlProblem, runSqliteQuery, SQL_MAX_ROWS, SQL_TIMEOUT_MS } from './g20-sql.ts'
import { openTunnel } from './ssh-remote.ts'

export interface DbResult {
  columns: string[]
  rows: string[][]
  truncated: boolean
  durationMs: number
}

const MAX_CELL_CHARS = 2000
const DEFAULT_PORTS = { postgres: 5432, mysql: 3306 } as const

/**
 * PostgreSQL okuması: $etiket$…$etiket$ metindir. Genel denetim bunu bilmez, `$$'$$; COMMIT; … $$'$$` gibi bir ifadeyi
 * tek metin sabiti sanırdı. Kapanmayan sabit/metin: null.
 */
function maskPostgres(sql: string): string | null {
  let out = ''
  let i = 0
  while (i < sql.length) {
    const ch = sql[i] ?? ''
    const dollar = ch === '$' && !/[\p{L}\p{N}_$]/u.test(sql[i - 1] ?? '') ? /^\$(?:[\p{L}_][\p{L}\p{N}_]*)?\$/u.exec(sql.slice(i, i + 70)) : null
    if (sql.startsWith('--', i)) {
      const end = sql.indexOf('\n', i)
      i = end === -1 ? sql.length : end
    } else if (sql.startsWith('/*', i)) {
      let depth = 1
      i += 2
      while (i < sql.length && depth > 0) {
        if (sql.startsWith('/*', i)) (depth++, (i += 2))
        else if (sql.startsWith('*/', i)) (depth--, (i += 2))
        else i++
      }
      if (depth > 0) return null
      out += ' '
    } else if (dollar) {
      const end = sql.indexOf(dollar[0], i + dollar[0].length)
      if (end === -1) return null
      out += "''"
      i = end + dollar[0].length
    } else if (ch === "'" || ch === '"') {
      let j = i + 1
      for (;;) {
        if (j >= sql.length) return null
        if (sql[j] === ch && sql[j + 1] === ch) j += 2
        else if (sql[j] === ch) break
        else j++
      }
      out += "''"
      i = j + 1
    } else {
      out += ch
      i++
    }
  }
  return out
}

const INTO_PROBLEM = 'SELECT … INTO (tablo/dosya/değişkene yazma) salt okunur modda çalıştırılmaz'

/** Sorgu salt okunur kurallara uymuyorsa nedeni; uyuyorsa null. */
export function dbQueryProblem(sql: string): string | null {
  const base = readOnlySqlProblem(sql)
  if (base) return base
  const outsideLiterals = sql.replace(/--[^\n]*|\/\*[\s\S]*?\*\//g, ' ').replace(/'(?:[^'\\]|\\.|'')*'|"(?:[^"]|"")*"|`[^`]*`/g, "''")
  if (/\binto\b/i.test(outsideLiterals)) return INTO_PROBLEM
  if (sql.includes('$')) {
    const pg = maskPostgres(sql)
    if (pg === null) return 'Kapanmayan metin sabiti'
    if (pg.trim().replace(/;\s*$/, '').includes(';')) return 'Tek seferde yalnızca bir sorgu çalıştırılır'
    if (/\binto\b/i.test(pg)) return INTO_PROBLEM
  }
  return null
}

/** Genişletilmiş protokol: sunucu tek ifade dışını reddeder; denetim atlatılsa bile COMMIT ile salt okunur işlemden çıkılamaz. */
export function postgresQuery(sql: string, maxRows: number): { text: string; rowMode: 'array'; queryMode: 'extended' } {
  return { text: limitQuery(sql, maxRows), rowMode: 'array', queryMode: 'extended' }
}

/** SELECT/WITH/VALUES dış sorguyla sınırlanır: büyük tablo belleği doldurmasın. EXPLAIN olduğu gibi kalır. */
export function limitQuery(sql: string, maxRows: number): string {
  const body = sql.trim().replace(/;\s*$/, '')
  if (!/^\s*(?:select|with|values)\b/i.test(body.replace(/^(?:\s|--[^\n]*\n|\/\*[\s\S]*?\*\/)*/, ''))) return body
  return `SELECT * FROM (\n${body}\n) AS kor_q LIMIT ${maxRows + 1}`
}

export function cellText(value: unknown): string {
  if (value === null || value === undefined) return 'NULL'
  if (value instanceof Date) return Number.isNaN(value.getTime()) ? String(value) : value.toISOString()
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) return `[ikili ${value.length} bayt]`
  const text = typeof value === 'object' ? JSON.stringify(value) : String(value)
  return text.length > MAX_CELL_CHARS ? `${text.slice(0, MAX_CELL_CHARS)}…` : text
}

function finish(columns: string[], rawRows: unknown[][], maxRows: number, started: number): DbResult {
  return {
    columns,
    rows: rawRows.slice(0, maxRows).map((row) => row.map(cellText)),
    truncated: rawRows.length > maxRows,
    durationMs: Date.now() - started,
  }
}

export interface DbQueryOptions {
  /** sqlite dosyası bu köke hapsedilmiş olarak verilmeli (çağıran resolveInWorkspace yapar) */
  sqliteFile?: string
  tunnel?: SshHost | null
  maxRows?: number
  timeoutMs?: number
}

async function streamFor(db: DbConnection, tunnel: SshHost | null | undefined): Promise<Duplex | undefined> {
  if (!tunnel) return undefined
  if (db.kind === 'sqlite') return undefined
  return openTunnel(tunnel, db.host ?? '127.0.0.1', db.port ?? DEFAULT_PORTS[db.kind])
}

async function queryPostgres(db: DbConnection, sql: string, o: Required<Pick<DbQueryOptions, 'maxRows' | 'timeoutMs'>> & { stream?: Duplex }): Promise<DbResult> {
  const { default: pg } = await import('pg')
  const password = revealSecret(db)
  const client = new pg.Client({
    host: db.host ?? '127.0.0.1',
    port: db.port ?? DEFAULT_PORTS.postgres,
    ...(db.database ? { database: db.database } : {}),
    ...(db.user ? { user: db.user } : {}),
    ...(password ? { password } : {}),
    ...(db.ssl ? { ssl: { rejectUnauthorized: true } } : {}),
    ...(o.stream ? { stream: () => o.stream } : {}),
    statement_timeout: o.timeoutMs,
    query_timeout: o.timeoutMs + 2000,
    connectionTimeoutMillis: 15_000,
    application_name: 'Kor (salt okunur)',
  })
  const started = Date.now()
  await client.connect()
  try {
    await client.query('BEGIN TRANSACTION READ ONLY')
    try {
      const res = await client.query(postgresQuery(sql, o.maxRows))
      return finish(res.fields.map((f) => f.name), res.rows as unknown[][], o.maxRows, started)
    } finally {
      await client.query('ROLLBACK').catch(() => undefined)
    }
  } finally {
    await client.end().catch(() => undefined)
  }
}

async function queryMysql(db: DbConnection, sql: string, o: Required<Pick<DbQueryOptions, 'maxRows' | 'timeoutMs'>> & { stream?: Duplex }): Promise<DbResult> {
  const mysql = await import('mysql2/promise')
  const password = revealSecret(db)
  const conn = await mysql.createConnection({
    host: db.host ?? '127.0.0.1',
    port: db.port ?? DEFAULT_PORTS.mysql,
    ...(db.database ? { database: db.database } : {}),
    ...(db.user ? { user: db.user } : {}),
    ...(password ? { password } : {}),
    ...(db.ssl ? { ssl: { rejectUnauthorized: true } } : {}),
    ...(o.stream ? { stream: o.stream } : {}),
    connectTimeout: 15_000,
    multipleStatements: false,
    dateStrings: true,
  })
  const started = Date.now()
  try {
    await conn.query('SET SESSION TRANSACTION READ ONLY')
    await conn.query('START TRANSACTION READ ONLY')
    try {
      const [rows, fields] = await conn.query({ sql: limitQuery(sql, o.maxRows), rowsAsArray: true, timeout: o.timeoutMs })
      const columns = Array.isArray(fields) ? fields.map((f) => f.name) : []
      return finish(columns, Array.isArray(rows) ? (rows as unknown[][]) : [], o.maxRows, started)
    } finally {
      await conn.query('ROLLBACK').catch(() => undefined)
    }
  } finally {
    await conn.end().catch(() => undefined)
  }
}

export async function dbQuery(db: DbConnection, sql: string, o: DbQueryOptions = {}): Promise<DbResult> {
  const problem = dbQueryProblem(sql)
  if (problem) throw new Error(problem)
  const maxRows = o.maxRows ?? SQL_MAX_ROWS
  const timeoutMs = o.timeoutMs ?? SQL_TIMEOUT_MS
  if (db.kind === 'sqlite') {
    if (!o.sqliteFile) throw new Error(`${db.id}: SQLite dosyası tanımlı değil.`)
    return runSqliteQuery(o.sqliteFile, sql, timeoutMs, maxRows)
  }
  const stream = await streamFor(db, o.tunnel)
  const opts = { maxRows, timeoutMs, ...(stream ? { stream } : {}) }
  try {
    return db.kind === 'postgres' ? await queryPostgres(db, sql, opts) : await queryMysql(db, sql, opts)
  } finally {
    stream?.destroy()
  }
}

/** Şema keşfi sorgusu: tablo/görünüm sütunları (sistem şemaları hariç). */
export function schemaSql(kind: DbConnection['kind']): string {
  switch (kind) {
    case 'postgres':
      return `SELECT table_schema || '.' || table_name AS table_name, column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_schema NOT IN ('pg_catalog', 'information_schema') AND table_schema NOT LIKE 'pg_toast%'
ORDER BY table_schema, table_name, ordinal_position`
    case 'mysql':
      return `SELECT table_name, column_name, column_type AS data_type, is_nullable
FROM information_schema.columns
WHERE table_schema = DATABASE()
ORDER BY table_name, ordinal_position`
    case 'sqlite':
      return `SELECT m.name AS table_name, p.name AS column_name, p.type AS data_type, CASE p."notnull" WHEN 1 THEN 'NO' ELSE 'YES' END AS is_nullable
FROM sqlite_master m JOIN pragma_table_info(m.name) p
WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%'
ORDER BY m.name, p.cid`
  }
}

/** "tablo(sütun tip, sütun? tip …)" satırları: modelin sorgu yazması için kısa şema. */
export function formatSchema(result: DbResult): string {
  const tables = new Map<string, string[]>()
  for (const [table = '?', column = '?', type = '', nullable = ''] of result.rows) {
    const cols = tables.get(table) ?? []
    cols.push(`${column}${nullable === 'YES' ? '?' : ''} ${type}`.trim())
    tables.set(table, cols)
  }
  if (tables.size === 0) return 'Tablo bulunamadı (veritabanı boş ya da kullanıcının yetkisi yok).'
  const lines = [...tables].map(([t, cols]) => `${t}(${cols.join(', ')})`)
  return `${tables.size} tablo/görünüm (? = NULL olabilir):\n${lines.join('\n')}${result.truncated ? '\n[şema satır sınırında kesildi]' : ''}`
}

export async function dbSchema(db: DbConnection, o: DbQueryOptions = {}): Promise<string> {
  return formatSchema(await dbQuery(db, schemaSql(db.kind), { ...o, maxRows: 5000 }))
}

/** Modele dönen tablo: sekmeyle ayrılmış, başlıklı; satır sayısı ve süre altta. */
export function formatDbResult(r: DbResult): string {
  if (r.columns.length === 0) return `Sorgu çalıştı, sonuç sütunu yok (${r.durationMs} ms).`
  const clean = (s: string) => s.replace(/[\t\r\n]+/g, ' ')
  const lines = [r.columns.map(clean).join('\t'), ...r.rows.map((row) => row.map(clean).join('\t'))]
  return `${lines.join('\n')}\n[${r.rows.length} satır${r.truncated ? ', sınırda kesildi' : ''} · ${r.durationMs} ms]`
}
