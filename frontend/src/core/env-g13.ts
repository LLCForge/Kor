// [g13] Ortam değişkeni yöneticisi (yalnızca ana süreç + arayüz). .env hassas yoldur (isSensitivePath): ajan araçları
// okuyamaz ve bu değişmez. Değerler burada günlüğe, denetim kaydına veya hata mesajına yazılmaz.
import { readdir, readFile, rename, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { confineToWorkspace } from './workspace.ts'

/** Yalnızca kökteki .env, .env.local, .env.production, .env.example ... (alt klasör ve yol ayracı yok). */
export const ENV_FILE_PATTERN = /^\.env(\.[\w-]{1,40})*$/
export const ENV_KEY_PATTERN = /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/
export const MAX_ENV_BYTES = 256_000
export const MAX_ENV_VALUE_CHARS = 10_000

export interface EnvEntry {
  key: string
  value: string
  /** 0 tabanlı satır */
  line: number
}

const LINE = /^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_.-]*)\s*=\s?(.*)$/

function unquote(raw: string): string {
  const v = raw.trimStart()
  if (v.startsWith('"')) {
    let out = ''
    for (let i = 1; i < v.length; i++) {
      const c = v[i]
      if (c === '"') return out
      if (c === '\\' && i + 1 < v.length) {
        const n = v[++i]
        out += n === 'n' ? '\n' : n === 'r' ? '\r' : n === 't' ? '\t' : (n ?? '')
      } else out += c
    }
    return out // Kapanmayan tırnak: satırın kalanı değerdir.
  }
  if (v.startsWith("'")) {
    const end = v.indexOf("'", 1)
    return end === -1 ? v.slice(1) : v.slice(1, end)
  }
  // Tırnaksız değerde boşlukla başlayan # satır içi yorumdur.
  return v.replace(/\s+#.*$/, '').trim()
}

export function parseEnv(text: string): EnvEntry[] {
  return text.split(/\r?\n/).flatMap((line, i) => {
    if (/^\s*#/.test(line)) return []
    const m = LINE.exec(line)
    return m?.[1] ? [{ key: m[1], value: unquote(m[2] ?? ''), line: i }] : []
  })
}

/** Güvenli karakterlerden oluşan değer tırnaksız; diğerleri kaçışlı çift tırnakla yazılır. */
export function serializeEnvValue(value: string): string {
  if (/^[\w.,:/@+%=-]*$/.test(value)) return value
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r')}"`
}

function assertKey(key: string): void {
  if (!ENV_KEY_PATTERN.test(key)) throw new Error('Geçersiz değişken adı (harf veya _ ile başlamalı; harf, rakam, _ . -)')
}

/** Anahtarın ilk satırını günceller, sonraki tekrarlarını siler; yoksa sona ekler. Diğer satırlar ve satır sonları korunur. */
export function setEnvValue(text: string, key: string, value: string): string {
  assertKey(key)
  if (value.length > MAX_ENV_VALUE_CHARS) throw new Error(`Değer en fazla ${MAX_ENV_VALUE_CHARS} karakter olabilir`)
  const eol = text.includes('\r\n') ? '\r\n' : '\n'
  const lines = text.length > 0 ? text.split(/\r?\n/) : []
  const hits = parseEnv(text).filter((e) => e.key === key).map((e) => e.line)
  const entry = `${key}=${serializeEnvValue(value)}`
  if (hits.length === 0) {
    if (lines.length > 0 && lines.at(-1) === '') lines.splice(-1, 1, entry, '')
    else lines.push(entry)
    return lines.join(eol)
  }
  const [first, ...rest] = hits
  return lines.flatMap((l, i) => (i === first ? [entry] : rest.includes(i) ? [] : [l])).join(eol)
}

export function removeEnvKey(text: string, key: string): string {
  assertKey(key)
  const hits = new Set(parseEnv(text).filter((e) => e.key === key).map((e) => e.line))
  const eol = text.includes('\r\n') ? '\r\n' : '\n'
  return text.split(/\r?\n/).filter((_, i) => !hits.has(i)).join(eol)
}

function envPath(root: string, name: string): string {
  if (!ENV_FILE_PATTERN.test(name)) throw new Error('Yalnızca kök klasördeki .env dosyaları yönetilebilir')
  return confineToWorkspace(root, name)
}

export async function listEnvFiles(root: string): Promise<string[]> {
  const entries = await readdir(root, { withFileTypes: true })
  return entries.filter((e) => e.isFile() && ENV_FILE_PATTERN.test(e.name)).map((e) => e.name).sort()
}

/** Dosya yoksa boş metin (ilk değişken eklenince oluşturulur). */
export async function readEnvFile(root: string, name: string): Promise<string> {
  const abs = envPath(root, name)
  try {
    const info = await stat(abs)
    if (!info.isFile()) throw new Error(`${name} bir dosya değil`)
    if (info.size > MAX_ENV_BYTES) throw new Error(`${name} çok büyük (sınır ${MAX_ENV_BYTES / 1000} KB)`)
    return await readFile(abs, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return ''
    throw err
  }
}

// ---------- Şablon (.env.example) ve eksik değişken denetimi ----------

/** Şablon sayılan dosyalar (ilk bulunan kullanılır). Değerleri gizli değildir; depoya girer. */
export const ENV_TEMPLATE_FILES = ['.env.example', '.env.template', '.env.sample', '.env.dist'] as const
/** Şablona karşı denetlenen gerçek dosyalar: .env.local, .env'i ezer (dotenv/Next/Vite düzeni). */
const ACTUAL_ENV_FILES = ['.env', '.env.local'] as const

export interface EnvTemplateReport {
  /** Bulunan şablon dosyası; yoksa null (denetim yapılmadı) */
  template: string | null
  /** Şablonda var, .env/.env.local'da hiç yok */
  missing: string[]
  /** Var ama değeri boş */
  empty: string[]
  /** .env'de var, şablonda yok (şablon güncel değil olabilir) */
  extra: string[]
}

/** Yalnızca ADLAR döner; değer hiçbir koşulda rapora girmez (ajan aracı da bunu kullanır). */
export async function envTemplateReport(root: string): Promise<EnvTemplateReport> {
  const files = await listEnvFiles(root)
  const template = ENV_TEMPLATE_FILES.find((name) => files.includes(name)) ?? null
  if (!template) return { template: null, missing: [], empty: [], extra: [] }
  const wanted = [...new Set(parseEnv(await readEnvFile(root, template)).map((e) => e.key))]
  const actual = new Map<string, string>()
  for (const name of ACTUAL_ENV_FILES) {
    if (!files.includes(name)) continue
    for (const e of parseEnv(await readEnvFile(root, name))) actual.set(e.key, e.value)
  }
  return {
    template,
    missing: wanted.filter((k) => !actual.has(k)),
    empty: wanted.filter((k) => actual.get(k) === ''),
    extra: [...actual.keys()].filter((k) => !wanted.includes(k)),
  }
}

/** .env'den şablon: her değer silinir (gizli bilgi depoya sızmasın); yorumlar, boş satırlar ve sıra korunur. */
export function templateFromEnv(text: string): string {
  const eol = text.includes('\r\n') ? '\r\n' : '\n'
  const keyed = new Map(parseEnv(text).map((e) => [e.line, e.key]))
  return text
    .split(/\r?\n/)
    .map((line, i) => {
      const key = keyed.get(i)
      return key ? `${/^\s*export\s/.test(line) ? 'export ' : ''}${key}=` : line
    })
    .join(eol)
}

/** Geçici dosya + taşıma: yazma yarıda kalırsa .env bozulmaz. */
export async function writeEnvFile(root: string, name: string, text: string): Promise<void> {
  const abs = envPath(root, name)
  if (Buffer.byteLength(text, 'utf8') > MAX_ENV_BYTES) throw new Error(`${name} çok büyük (sınır ${MAX_ENV_BYTES / 1000} KB)`)
  const tmp = path.join(path.dirname(abs), `${name}.${process.pid}.tmp`)
  await writeFile(tmp, text, { encoding: 'utf8', mode: 0o600 })
  await rename(tmp, abs)
}
