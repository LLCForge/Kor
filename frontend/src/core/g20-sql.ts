// [g20] SQL sonucu tablosu: yalnızca kullanıcının belirttiği, çalışma dizinindeki SQLite dosyası; SALT OKUNUR.
//      Sürücü olarak Node'un yerleşik node:sqlite modülü kullanılır (yeni bağımlılık yok). Sorgu ayrı bir alt süreçte
//      (Electron'da ELECTRON_RUN_AS_NODE ile aynı ikili) çalışır: ana süreç donmaz ve zaman aşımında süreç gerçekten
//      öldürülür (worker_thread yerel SQLite çağrısının ortasında sonlandırılamaz).
//      Katmanlar: başlangıç anahtar sözcüğü allowlist'i + tek ifade + readOnly bağlantı (yazma/ATTACH dosya yaratamaz)
//      + eklenti yükleme kapalı (node:sqlite varsayılanı) + satır ve hücre sınırı.
import { runProcess } from './g20-run.ts'

export const SQL_TIMEOUT_MS = 15_000
export const SQL_MAX_ROWS = 1000
export const SQL_FILE_EXTENSIONS = ['.db', '.sqlite', '.sqlite3', '.db3'] as const
const MAX_RESULT_CHARS = 5_000_000

export interface G20SqlResult {
  columns: string[]
  rows: string[][]
  truncated: boolean
  durationMs: number
}

/**
 * Yorumları boşluğa, metin sabitlerini '' ye çevirir. Regex yerine sırayla tarama: "--" sabitin içindeyse yorum değildir,
 * yorumdaki tırnak da sabit açmaz. null: kapanmayan sabit. backslash: MySQL'in \' kaçışı (SQLite'ta yok).
 */
function maskSql(sql: string, backslash: boolean): string | null {
  const closers: Record<string, string> = { "'": "'", '"': '"', '`': '`', '[': ']' }
  let out = ''
  let i = 0
  while (i < sql.length) {
    const ch = sql[i] ?? ''
    if (sql.startsWith('--', i)) {
      const end = sql.indexOf('\n', i)
      i = end === -1 ? sql.length : end
      out += ' '
    } else if (sql.startsWith('/*', i)) {
      const end = sql.indexOf('*/', i + 2)
      i = end === -1 ? sql.length : end + 2
      out += ' '
    } else if (closers[ch]) {
      const close = closers[ch]
      let j = i + 1
      for (;;) {
        if (j >= sql.length) return null
        if (backslash && (ch === "'" || ch === '"') && sql[j] === '\\') j += 2
        else if (sql[j] === close && ch !== '[' && ch !== '`' && sql[j + 1] === close) j += 2
        else if (sql[j] === close) break
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

/** Sorgu salt okunur allowlist'e uymuyorsa nedenini, uyuyorsa null döndürür. */
export function readOnlySqlProblem(sql: string): string | null {
  const standard = maskSql(sql, false)
  if (standard === null) return 'Kapanmayan metin sabiti'
  const stripped = standard.trim().replace(/;\s*$/, '')
  if (!stripped) return 'Sorgu boş'
  if (!/^(?:select|with|values|explain)\b/i.test(stripped)) return 'Yalnızca SELECT, WITH, VALUES veya EXPLAIN sorguları çalıştırılır'
  // db-remote MySQL'e de gider: sabitin nerede bittiği iki yorumda da aynı değilse ikisi de denetlenir, biri ; görürse reddedilir.
  const escaped = sql.includes('\\') ? maskSql(sql, true) : standard
  // escaped null: o yorumda sabit kapanmıyor; sunucu sözdizimi hatası verir, ikinci ifade çalışamaz ('C:\' SQLite'ta geçerlidir).
  const candidates = [stripped, (escaped ?? '').trim().replace(/;\s*$/, '')]
  if (candidates.some((text) => text.includes(';'))) return 'Tek seferde yalnızca bir sorgu çalıştırılır'
  return null
}

// Alt süreç programı: girdiyi stdin'den JSON olarak alır, sonucu stdout'a tek JSON olarak yazar.
const CHILD_SOURCE = `
const { DatabaseSync } = require('node:sqlite')
let input = ''
process.stdin.setEncoding('utf8')
process.stdin.on('data', (d) => { input += d })
process.stdin.on('end', () => {
  let db
  try {
    const { file, sql, maxRows } = JSON.parse(input)
    db = new DatabaseSync(file, { readOnly: true })
    const stmt = db.prepare(sql)
    if (typeof stmt.setReturnArrays === 'function') stmt.setReturnArrays(true)
    const columns = stmt.columns().map((c) => String(c.name))
    const cell = (v) => v === null || v === undefined ? 'NULL' : v instanceof Uint8Array ? '[BLOB ' + v.length + ' bayt]' : String(v).slice(0, 1000)
    const rows = []
    let truncated = false
    for (const row of stmt.iterate()) {
      if (rows.length >= maxRows) { truncated = true; break }
      rows.push(Array.isArray(row) ? row.map(cell) : columns.map((c) => cell(row[c])))
    }
    process.stdout.write(JSON.stringify({ ok: true, columns, rows, truncated }))
  } catch (e) {
    process.stdout.write(JSON.stringify({ ok: false, error: String((e && e.message) || e) }))
  } finally {
    if (db) db.close()
  }
})
`

const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === 'string')

/** Sorguyu salt okunur bağlantıyla çalıştırır. Dosya yolu çağıran tarafından çalışma dizinine hapsedilmiş olmalıdır. */
export async function runSqliteQuery(file: string, sql: string, timeoutMs = SQL_TIMEOUT_MS, maxRows = SQL_MAX_ROWS): Promise<G20SqlResult> {
  const problem = readOnlySqlProblem(sql)
  if (problem) throw new Error(problem)
  const result = await runProcess(process.execPath, ['--no-warnings', '-e', CHILD_SOURCE], {
    cwd: process.cwd(),
    stdin: JSON.stringify({ file, sql, maxRows }),
    // Electron'da aynı ikili düz Node olarak çalışır; düz Node bu değişkeni yok sayar. Gizli anahtar aktarılmaz.
    env: { PATH: process.env.PATH, SYSTEMROOT: process.env.SYSTEMROOT, ELECTRON_RUN_AS_NODE: '1' },
    timeoutMs,
    maxOutput: MAX_RESULT_CHARS,
  })
  if (result.timedOut) throw new Error(`Sorgu ${timeoutMs / 1000} sn içinde bitmedi ve durduruldu`)
  if (result.truncated) throw new Error('Sorgu sonucu çok büyük')
  const stdout = result.chunks
    .filter((c) => c.stream === 'out')
    .map((c) => c.text)
    .join('')
  let data: { ok?: unknown; error?: unknown; columns?: unknown; rows?: unknown; truncated?: unknown }
  try {
    data = JSON.parse(stdout) as typeof data
  } catch {
    const stderr = result.chunks.filter((c) => c.stream === 'err').map((c) => c.text).join('')
    console.warn('SQL alt süreci geçersiz çıktı verdi:', stderr.slice(0, 2000))
    throw new Error(`Sorgu çalıştırılamadı (çıkış kodu ${result.exitCode ?? '?'})`)
  }
  if (data.ok !== true) throw new Error(`Sorgu başarısız: ${typeof data.error === 'string' ? data.error.slice(0, 500) : 'bilinmeyen hata'}`)
  if (!isStringArray(data.columns) || !Array.isArray(data.rows) || !data.rows.every(isStringArray)) throw new Error('Sorgu sonucu beklenmeyen biçimde')
  return { columns: data.columns, rows: data.rows, truncated: data.truncated === true, durationMs: result.durationMs }
}
