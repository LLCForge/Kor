// [g13] Yığın izi gezgini ve hata açıklayıcı için saf yardımcılar (DOM yok; test edilebilir).

export interface StackRef {
  /** Metin içindeki bağlantı aralığı */
  start: number
  end: number
  /** Çalışma dizinine göre, '/' ayraçlı */
  path: string
  line: number
}

const EXT = String.raw`\.[A-Za-z]\w{0,9}`
const SEG = String.raw`[\w@$~+-][\w@.$~+-]*`
const PLAIN_PATH = String.raw`(?:file:\/\/\/?)?(?:[A-Za-z]:[\\/]|[\\/]|\.{1,2}[\\/])?(?:${SEG}[\\/])*${SEG}${EXT}`
// URL'nin ortasında (://host/a.js) veya bir kelimenin içinde başlamasın.
const LEAD = String.raw`(?<![\w:/\\@.$-])`

/** re: grup 1 yol, grup 2 satır. trimParens: bağlantı aralığına çevreleyen parantezler girmez. */
interface Pattern {
  re: RegExp
  trimParens?: boolean
}

// Öncelik sırası: önce özel biçimler; genel "yol:satır" en son ve çakışanlar atlanır.
const PATTERNS: Pattern[] = [
  { re: /File "([^"\n]+)", line (\d+)/g }, // Python
  { re: new RegExp(String.raw`\(((?:file:\/\/\/?)?[^()\n]*?${EXT}):(\d+)(?::\d+)?\)`, 'g'), trimParens: true }, // Node: at fn (C:\a b\x.ts:1:2)
  { re: new RegExp(String.raw`${LEAD}(${PLAIN_PATH})\((\d+),\d+\)`, 'g') }, // tsc: src\a.ts(12,5)
  { re: new RegExp(String.raw`${LEAD}(${PLAIN_PATH}):line (\d+)`, 'g') }, // .NET: in C:\x\Foo.cs:line 42
  { re: new RegExp(String.raw`${LEAD}(${PLAIN_PATH}):(\d+)(?::\d+)?`, 'g') }, // gcc, rustc, go, eslint, Node
]

/**
 * Yığın izindeki yolu çalışma dizinine göre göreli yola çevirir. Kök dışı, "..", sürücü/protokol içeren
 * (node:internal/...) yollar null döner: bağlantı yalnızca çalışma dizinindeki dosyalara açılır.
 */
export function toWorkspacePath(raw: string, root: string): string | null {
  let p = raw.trim()
  if (/^file:\/\//i.test(p)) {
    try {
      p = decodeURIComponent(p.replace(/^file:\/\/\/?/i, ''))
    } catch {
      return null
    }
    if (!/^[A-Za-z]:/.test(p)) p = `/${p}`
  }
  p = p.replaceAll('\\', '/')
  const base = root.replaceAll('\\', '/').replace(/\/+$/, '')
  const windows = /^[A-Za-z]:/.test(base)
  let rel: string
  if (/^[A-Za-z]:\//.test(p) || p.startsWith('/')) {
    const fold = (s: string) => (windows ? s.toLowerCase() : s)
    if (!fold(p).startsWith(`${fold(base)}/`)) return null
    rel = p.slice(base.length + 1)
  } else rel = p
  const parts = rel.split('/').filter((s) => s !== '.')
  if (parts.length === 0 || parts.some((s) => s === '' || s === '..' || /[:*?"<>|]/.test(s))) return null
  return parts.join('/')
}

export function findStackRefs(text: string, root: string): StackRef[] {
  const refs: StackRef[] = []
  const overlaps = (start: number, end: number) => refs.some((r) => start < r.end && end > r.start)
  for (const { re, trimParens } of PATTERNS) {
    for (const m of text.matchAll(re)) {
      const raw = m[1]
      const line = Number(m[2])
      if (!raw || m.index === undefined || !Number.isInteger(line) || line < 1) continue
      const start = trimParens ? m.index + 1 : m.index
      const end = trimParens ? m.index + m[0].length - 1 : m.index + m[0].length
      if (overlaps(start, end)) continue
      const path = toWorkspacePath(raw, root)
      if (path) refs.push({ start, end, path, line })
    }
  }
  return refs.sort((a, b) => a.start - b.start)
}

export const MAX_EXPLAIN_CHARS = 8_000

/** "Açıkla ve çöz": seçili hata metnini sohbete hazır istem olarak taşır (gönderilmeden önce kullanıcı düzenleyebilir). */
export function explainPrompt(selection: string): string {
  const body = selection.trim()
  const text = body.length > MAX_EXPLAIN_CHARS ? `${body.slice(0, MAX_EXPLAIN_CHARS)}\n[kesildi]` : body
  const fence = text.includes('```') ? '~~~~' : '```'
  return [
    'Aşağıdaki hatayı açıkla ve çöz: kök nedeni bul, ilgili dosyaları (yığın izindeki dosya:satır konumları dahil) oku, düzeltmeyi uygula ve nasıl doğruladığını söyle.',
    '',
    `${fence}text`,
    text,
    fence,
  ].join('\n')
}
