// [g20] Satır içi JS/TS REPL'in saf çekirdeği (DOM yok, node:test ile test edilir).
//
// Model kodu ana belgede ÇALIŞMAZ: kullanıcı kodu, sertleştirme önsözüyle birlikte bir blob: Worker betiğinin
// gövdesine yerleştirilir (bkz. g20-sandbox.ts). eval/new Function kullanılmaz; blob worker ana belgenin CSP'sini
// devraldığından eval zaten kapalıdır. Önsöz ve biçimleyici, worker'a Function.prototype.toString ile aktarılır;
// bu yüzden ikisi de KENDİ İÇİNDE KAPALI olmalıdır (dış modül bağlamına referans yok).

/** Worker'dan ana tarafa gelen değişken özeti. */
export interface VarInfo {
  name: string
  type: string
  preview: string
}

/**
 * Değeri okunabilir, sınırlı metne çevirir (Node REPL benzeri). Döngüsel yapı, getter hatası ve Proxy tuzakları
 * güvenle ele alınır. KENDİ İÇİNDE KAPALI: worker'a toString ile kopyalanır.
 */
export function formatValue(value: unknown, depth = 0, seen: object[] = []): string {
  const MAX_ITEMS = 50
  const kind = typeof value
  if (value === null) return 'null'
  if (kind === 'string') return depth === 0 ? (value as string) : JSON.stringify(value)
  if (kind === 'number' || kind === 'boolean' || kind === 'undefined') return String(value)
  if (kind === 'bigint') return `${String(value)}n`
  if (kind === 'symbol') return (value as symbol).toString()
  if (kind === 'function') {
    const name = (value as { name?: unknown }).name
    return `[Function ${typeof name === 'string' && name ? name : '(anonim)'}]`
  }
  const obj = value as object
  if (seen.includes(obj)) return '[Döngüsel]'
  try {
    if (obj instanceof Error) return `${obj.name}: ${obj.message}`
    if (obj instanceof Date) return Number.isNaN(obj.getTime()) ? 'Invalid Date' : obj.toISOString()
    if (obj instanceof RegExp) return String(obj)
    if (obj instanceof Promise) return 'Promise { <bekliyor> }'
    if (depth > 3) return Array.isArray(obj) ? '[Array]' : '[Object]'
    const next = [...seen, obj]
    const more = (count: number) => (count > MAX_ITEMS ? [`… ${count - MAX_ITEMS} öğe daha`] : [])
    if (Array.isArray(obj)) {
      const items = obj.slice(0, MAX_ITEMS).map((item) => formatValue(item, depth + 1, next))
      return `[${[...items, ...more(obj.length)].join(', ')}]`
    }
    if (obj instanceof Map) {
      const items = [...obj.entries()].slice(0, MAX_ITEMS).map(([k, v]) => `${formatValue(k, depth + 1, next)} => ${formatValue(v, depth + 1, next)}`)
      return `Map(${obj.size}) {${[...items, ...more(obj.size)].join(', ')}}`
    }
    if (obj instanceof Set) {
      const items = [...obj.values()].slice(0, MAX_ITEMS).map((v) => formatValue(v, depth + 1, next))
      return `Set(${obj.size}) {${[...items, ...more(obj.size)].join(', ')}}`
    }
    if (ArrayBuffer.isView(obj)) {
      const view = obj as unknown as { length?: number; byteLength: number }
      return `${obj.constructor.name}(${view.length ?? view.byteLength})`
    }
    const ctor = (obj as { constructor?: { name?: unknown } }).constructor?.name
    const prefix = typeof ctor === 'string' && ctor && ctor !== 'Object' ? `${ctor} ` : ''
    const keys = Object.keys(obj)
    const entries = keys.slice(0, MAX_ITEMS).map((key) => {
      const label = /^[A-Za-z_$][\w$]*$/.test(key) ? key : JSON.stringify(key)
      let shown: string
      try {
        shown = formatValue((obj as Record<string, unknown>)[key], depth + 1, next)
      } catch (err) {
        shown = `[okunamadı: ${err instanceof Error ? err.message : 'hata'}]`
      }
      return `${label}: ${shown}`
    })
    return `${prefix}{${[...entries, ...more(keys.length)].join(', ')}}`
  } catch (err) {
    return `[biçimlenemedi: ${err instanceof Error ? err.message : 'hata'}]`
  }
}

/** Worker içinde kaldırılan yetenekler: ağ, kalıcı depolama, iç içe worker ve ana tarafa sahte mesaj. */
export const BANNED_WORKER_GLOBALS = [
  'fetch',
  'XMLHttpRequest',
  'WebSocket',
  'WebSocketStream',
  'EventSource',
  'WebTransport',
  'importScripts',
  'Worker',
  'SharedWorker',
  'BroadcastChannel',
  'indexedDB',
  'caches',
  'Notification',
  'postMessage',
] as const

/**
 * Worker içinde kullanıcı kodundan ÖNCE çalışan önsöz (savunmanın ikinci katmanı; birinci katman blob worker'ın
 * devraldığı CSP'dir). Ağ/depolama global'lerini prototip zinciri boyunca siler, console'u sınırlı çıktıya yönlendirir
 * ve self.__g20 üzerinden çalıştırma/değişken yakalama işlevlerini sunar. KENDİ İÇİNDE KAPALI: toString ile kopyalanır.
 */
export function workerPrelude(fmt: (value: unknown, depth?: number) => string, banned: readonly string[], maxOutput: number): void {
  const scope = self as unknown as Record<string, unknown>
  const post = (scope.postMessage as (message: unknown) => void).bind(scope)
  for (const name of banned) {
    let target: object | null = scope
    while (target) {
      try {
        delete (target as Record<string, unknown>)[name]
      } catch {
        // Yapılandırılamayan özellik: aşağıdaki kalan kontrolü raporlar.
      }
      target = Object.getPrototypeOf(target) as object | null
    }
  }
  try {
    delete (Object.getPrototypeOf(scope.navigator) as Record<string, unknown>).storage
  } catch {
    // navigator.storage yoksa yapılacak bir şey yok.
  }
  const remaining = banned.filter((name) => typeof scope[name] !== 'undefined')

  let used = 0
  let buffer: { stream: 'out' | 'err'; text: string }[] = []
  let truncated = false
  let timer: ReturnType<typeof setTimeout> | undefined
  const flush = () => {
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
    if (buffer.length > 0) post({ type: 'out', chunks: buffer, truncated })
    buffer = []
  }
  const emit = (stream: 'out' | 'err', args: unknown[]) => {
    if (truncated) return
    let text = `${args.map((a) => fmt(a)).join(' ')}\n`
    if (used + text.length > maxOutput) {
      text = text.slice(0, Math.max(0, maxOutput - used))
      truncated = true
    }
    used += text.length
    const last = buffer[buffer.length - 1]
    if (last && last.stream === stream) last.text += text
    else buffer.push({ stream, text })
    // Sıkı döngüde de çıktı kaybolmasın diye dolu tampon hemen gönderilir; kalanı bir sonraki tikte.
    if (truncated || buffer.reduce((n, c) => n + c.text.length, 0) >= 4096) flush()
    else if (timer === undefined) timer = setTimeout(flush, 0)
  }
  const out = (...args: unknown[]) => emit('out', args)
  const err = (...args: unknown[]) => emit('err', args)
  Object.defineProperty(scope, 'console', {
    value: { log: out, info: out, debug: out, dir: out, table: out, trace: err, warn: err, error: err, assert: (ok: unknown, ...a: unknown[]) => (ok ? undefined : err('Assertion failed', ...a)) },
    configurable: true,
    writable: true,
  })

  const typeName = (v: unknown) => {
    if (v === null) return 'null'
    if (Array.isArray(v)) return 'Array'
    if (typeof v === 'object') return (v as { constructor?: { name?: string } }).constructor?.name ?? 'Object'
    return typeof v
  }
  let vars: { name: string; type: string; preview: string }[] = []
  scope.__g20 = {
    vars(getters: Record<string, () => unknown>) {
      vars = []
      for (const [name, get] of Object.entries(getters)) {
        try {
          const v = get()
          if (typeof v === 'function') continue
          vars.push({ name, type: typeName(v), preview: fmt(typeof v === 'string' ? JSON.stringify(v) : v).slice(0, 300) })
        } catch {
          // Henüz tanımlanmamış (TDZ) ya da blok kapsamlı ad: listeye girmez.
        }
      }
    },
    async run(main: () => Promise<unknown>) {
      post({ type: 'started', remaining })
      const started = performance.now()
      try {
        const result = await main()
        flush()
        let json: string | undefined
        try {
          json = result === undefined ? undefined : JSON.stringify(result)
          if (json !== undefined && json.length > 500_000) json = undefined
        } catch {
          json = undefined
        }
        const shown = result === undefined ? undefined : fmt(typeof result === 'string' ? JSON.stringify(result) : result)
        post({ type: 'done', ok: true, result: shown?.slice(0, maxOutput), json, vars, ms: performance.now() - started })
      } catch (e) {
        flush()
        const error = e instanceof Error ? `${e.name}: ${e.message}` : `Yakalanmamış değer: ${fmt(e)}`
        post({ type: 'done', ok: false, error, vars, ms: performance.now() - started })
      }
    },
  }
}

const RESERVED = new Set(
  'await break case catch class const continue debugger default delete do else enum export extends false finally for function if implements import in instanceof interface let new null package private protected public return static super switch this throw true try typeof var void while with yield'.split(
    ' ',
  ),
)
const IDENT = /^[A-Za-z_$][\w$]*$/

/** Destructuring kalıbındaki (`{a, b: c, ...d}` / `[x, , y = 1]`) bağlanan adlar. */
function patternNames(pattern: string): string[] {
  return pattern
    .slice(1, -1)
    .split(',')
    .map((part) => {
      const target = part.includes(':') ? part.slice(part.indexOf(':') + 1) : part
      return target.replace(/=.*$/s, '').replace(/^\s*\.\.\./, '').trim()
    })
    .filter((name) => IDENT.test(name))
}

/**
 * Üst düzey (girintisiz) bildirimlerin adları: değişken denetçisi bu adları worker'da okur. Sezgiseldir; kaçan ad
 * yalnızca denetçide görünmez. Çok satırlı tek bildirimde yalnızca ilk bildirici yakalanır.
 */
export function topLevelNames(code: string): string[] {
  const names = new Set<string>()
  for (const line of code.split('\n')) {
    const decl = /^(?:export\s+)?(?:const|let|var)\s+(\{[^}]*\}|\[[^\]]*\]|[A-Za-z_$][\w$]*)/.exec(line)
    if (decl?.[1]) {
      const target = decl[1]
      for (const name of target.startsWith('{') || target.startsWith('[') ? patternNames(target) : [target]) names.add(name)
      continue
    }
    const fn = /^(?:export\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)/.exec(line) ?? /^(?:export\s+)?class\s+([A-Za-z_$][\w$]*)/.exec(line)
    if (fn?.[1]) names.add(fn[1])
  }
  return [...names].filter((n) => IDENT.test(n) && !RESERVED.has(n) && !n.startsWith('__g20')).slice(0, 50)
}

const STATEMENT_START =
  /^(?:const|let|var|function|async\s+function|class|if|else|for|while|do|switch|case|default|try|catch|finally|return|throw|break|continue|import|export|debugger)\b|^[})\]]|^\/[/*]|^\*/
const CONTINUES = /[{([,=+\-*/%&|^!?:<>.]$/

/**
 * Son üst düzey satır tek başına bir ifade gibi görünüyorsa onu ayırır (REPL'in "son ifadenin değeri"). Yanlış
 * tahmin zararsızdır: sözdizimi hatası verirse çalıştırıcı ifadesiz kipe düşer (kod hiç çalışmadan ayrıştırma hatası).
 */
export function splitLastExpression(code: string): { head: string; last: string } | null {
  const lines = code.replace(/\s+$/, '').split('\n')
  let index = lines.length - 1
  while (index >= 0 && (lines[index]?.trim() === '' || lines[index]?.trim().startsWith('//'))) index--
  const line = lines[index]
  if (line === undefined || /^\s/.test(line)) return null
  const last = line.trim().replace(/;\s*(?:\/\/.*)?$/, '').trim()
  if (!last || STATEMENT_START.test(last) || CONTINUES.test(last)) return null
  const previous = lines
    .slice(0, index)
    .reverse()
    .find((l) => l.trim() !== '')
  if (previous !== undefined && CONTINUES.test(previous.replace(/\/\/.*$/, '').trim())) return null
  return { head: lines.slice(0, index).join('\n'), last }
}

export interface ScriptMode {
  /** Son ifadenin değerini sonuç olarak döndür */
  returnLast: boolean
  /** Üst düzey adları değişken denetçisi için yakala */
  captureVars: boolean
}

/** Sözdizimi hatasında sırayla denenen kipler: en zengin olandan en yalına. */
export const SCRIPT_MODES: readonly ScriptMode[] = [
  { returnLast: true, captureVars: true },
  { returnLast: false, captureVars: true },
  { returnLast: false, captureVars: false },
]

/**
 * Worker betiği: önsöz + kullanıcı kodu (async fonksiyon gövdesi; üst düzey await çalışır). Önsöz ve biçimleyici
 * kaynakları toString ile gömülür; kullanıcı kodu metin olarak eklenir ve hiçbir yerde eval edilmez.
 */
export function buildWorkerScript(code: string, mode: ScriptMode, maxOutput: number): string {
  const names = mode.captureVars ? topLevelNames(code) : []
  // Kullanıcı `self` adını gölgeleyebilir; global özellik çıplak adla okunur.
  const capture = names.length > 0 ? `__g20.vars({${names.map((n) => `${n}: () => ${n}`).join(', ')}});` : ''
  const split = mode.returnLast ? splitLastExpression(code) : null
  const body = split
    ? `${split.head}\nconst __g20_result = (\n${split.last}\n);\n${capture}\nreturn __g20_result;`
    : `${code}\n;${capture}`
  return [
    `(${workerPrelude.toString()})(${formatValue.toString()}, ${JSON.stringify(BANNED_WORKER_GLOBALS)}, ${maxOutput});`,
    'self.__g20.run(async function () {',
    body,
    '});',
  ].join('\n')
}

/** CSV hücresi: virgül, tırnak veya satır sonu içeriyorsa tırnaklanır. */
function csvCell(value: unknown): string {
  const text = value === null || value === undefined ? '' : typeof value === 'object' ? JSON.stringify(value) : String(value)
  return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text
}

const MAX_CHART_ROWS = 5000
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)

/**
 * REPL sonucunu (JSON) grafik bloğu girdisine (başlıklı CSV) çevirir: sayı dizisi, [x, y] çiftleri, sayısal alanlı
 * nesne dizisi ya da etiket → sayı nesnesi. Grafiğe uygun değilse null.
 */
export function toChartCsv(value: unknown): string | null {
  if (Array.isArray(value)) {
    if (value.length < 2 || value.length > MAX_CHART_ROWS) return null
    if (value.every(isNum)) return ['x,değer', ...value.map((v, i) => `${i},${v}`)].join('\n')
    if (value.every((p) => Array.isArray(p) && p.length === 2 && isNum(p[1]) && (isNum(p[0]) || typeof p[0] === 'string'))) {
      return ['x,y', ...value.map((p) => `${csvCell((p as unknown[])[0])},${(p as unknown[])[1]}`)].join('\n')
    }
    if (value.every((row) => row !== null && typeof row === 'object' && !Array.isArray(row))) {
      const rows = value as Record<string, unknown>[]
      const headers: string[] = []
      for (const row of rows) for (const key of Object.keys(row)) if (!headers.includes(key) && headers.length < 12) headers.push(key)
      if (!headers.some((h) => rows.every((row) => isNum(row[h])))) return null
      return [headers.map(csvCell).join(','), ...rows.map((row) => headers.map((h) => csvCell(row[h])).join(','))].join('\n')
    }
    return null
  }
  if (value !== null && typeof value === 'object') {
    const entries = Object.entries(value)
    if (entries.length < 2 || entries.length > MAX_CHART_ROWS || !entries.every(([, v]) => isNum(v))) return null
    return ['etiket,değer', ...entries.map(([k, v]) => `${csvCell(k)},${String(v)}`)].join('\n')
  }
  return null
}
