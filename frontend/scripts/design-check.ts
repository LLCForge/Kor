// Tasarım denetimi (spec §16.2 kural 1, §23.5, ADR 0053). İki kural:
//  1. Tema hex'leri yalnızca assets/theme.json ve ondan üretilen dosyalarda yaşar. Kaynak kodda elle kopyalanmış
//     bir tema rengi (ör. ember.mid hex değeri, Rust'ta (r, g, b) ya da ANSI 38;2;r;g;b) reddedilir.
//  2. Arayüzde yeni dosyalar serbest değer kullanmaz: CSS/TSX'te ham `#hex` ya da `13px` yerine token
//     (var(--kor-…)). Bu kuraldan önce var olan dosyalar scripts/ui-lint-allowlist.json'dadır; liste yalnızca küçülür.
// `node scripts/design-check.ts` raporlar (hata varsa 1 ile çıkar); `--write-allowlist` listeyi yeniden üretir.
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { extname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { OUT, ROOT, THEME_PATH, rgbOf, type Theme } from './theme-gen.ts'

export const ALLOWLIST_PATH = resolve(ROOT, 'frontend/scripts/ui-lint-allowlist.json')
export const RENDERER = resolve(ROOT, 'frontend/src/desktop/renderer')

/** Tema kopyası aranan kökler (kod; belgeler ve spec değil). */
const THEME_ROOTS = ['assets', 'frontend/src', 'frontend/extension', 'frontend/scripts', 'frontend/src-tauri/src', 'backend/apps', 'backend/crates']
const SKIP_DIRS = new Set(['node_modules', 'target', 'dist', 'gen', '.git', 'okuma_bunu_llm'])
const CODE_EXT = new Set(['.ts', '.tsx', '.js', '.mjs', '.css', '.html', '.json', '.rs', '.toml'])

/** Tema değerlerinin yaşamasına izin verilen dosyalar: kaynak ve üretilen çıktılar. */
// design-check.test.ts spec değerlerini (§23.1) bilerek sınar; denetimin kendi test sabitidir.
export const THEME_ALLOWED = new Set(
  [THEME_PATH, OUT.ts, OUT.css, OUT.extCss, OUT.rust, OUT.tauri, resolve(ROOT, 'frontend/scripts/design-check.test.ts')].map((p) => resolve(p)),
)

export interface Violation {
  file: string
  line: number
  value: string
}

export function walk(dir: string, exts: ReadonlySet<string>, out: string[] = []): string[] {
  let entries: string[]
  try {
    entries = readdirSync(dir)
  } catch {
    return out
  }
  for (const name of entries) {
    if (SKIP_DIRS.has(name)) continue
    const p = resolve(dir, name)
    const st = statSync(p)
    if (st.isDirectory()) walk(p, exts, out)
    else if (exts.has(extname(name)) && st.size < 2_000_000) out.push(p)
  }
  return out
}

export function themeNeedles(t: Theme): { hex: Set<string>; rgb: RegExp[] } {
  const all = [...Object.values(t.ember), ...Object.values(t.ink), ...Object.values(t.status), ...t.gradient]
  const hex = new Set(all.map((h) => h.toUpperCase()))
  const rgb = [...hex].flatMap((h) => {
    const [r, g, b] = rgbOf(h)
    return [new RegExp(`\\(\\s*${r}\\s*,\\s*${g}\\s*,\\s*${b}\\s*\\)`), new RegExp(`(?<!\\d)${r};${g};${b}(?!\\d)`)]
  })
  return { hex, rgb }
}

/** Bir dosya metnindeki elle kopyalanmış tema renkleri. */
export function themeCopies(text: string, file: string, needles: ReturnType<typeof themeNeedles>): Violation[] {
  const out: Violation[] = []
  const lines = text.split('\n')
  lines.forEach((line, i) => {
    for (const m of line.matchAll(/#[0-9A-Fa-f]{6}\b/g)) {
      if (needles.hex.has(m[0].toUpperCase())) out.push({ file, line: i + 1, value: m[0] })
    }
    if (file.endsWith('.rs')) for (const re of needles.rgb) {
      const m = line.match(re)
      if (m) out.push({ file, line: i + 1, value: m[0] })
    }
  })
  return out
}

export function scanThemeCopies(): Violation[] {
  const theme = JSON.parse(readFileSync(THEME_PATH, 'utf8')) as Theme
  const needles = themeNeedles(theme)
  return THEME_ROOTS.flatMap((r) => walk(resolve(ROOT, r), CODE_EXT))
    .filter((f) => !THEME_ALLOWED.has(resolve(f)))
    .flatMap((f) => themeCopies(readFileSync(f, 'utf8'), relative(ROOT, f).replaceAll('\\', '/'), needles))
}

// `1px` (ince kenarlık, spec §23.1) ve sıfır serbest; diğer tüm px değerleri token'dan gelir.
const RAW_PX = /(?<![\w-])(?!0px|1px\b)\d+(?:\.\d+)?px\b/g
const RAW_HEX = /#[0-9A-Fa-f]{3,8}\b/g

/** CSS/TSX metnindeki serbest değerler (yorum satırları hariç). */
export function rawValues(text: string, file: string): Violation[] {
  const out: Violation[] = []
  text.split('\n').forEach((line, i) => {
    const code = line.replace(/\/\*.*?\*\//g, '').replace(/^\s*(\/\/|\*|\/\*).*$/, '')
    // Medya/kapsayıcı sorgularında var() kullanılamaz; kırılma noktası theme.json layout değeriyle aynı tutulur.
    if (/^\s*@(media|container)\s/.test(code)) return
    for (const re of [RAW_HEX, RAW_PX]) for (const m of code.matchAll(re)) {
      // JSX'te `#` ile başlayan kimlik bağlantıları (href="#x") hex değildir.
      if (re === RAW_HEX && /href=["']#$/.test(code.slice(0, m.index! + 1))) continue
      out.push({ file, line: i + 1, value: m[0] })
    }
  })
  return out
}

export function uiFiles(): string[] {
  return walk(RENDERER, new Set(['.css', '.tsx']))
    .filter((f) => !THEME_ALLOWED.has(resolve(f)))
    .map((f) => relative(ROOT, f).replaceAll('\\', '/'))
}

export function readAllowlist(): Set<string> {
  try {
    return new Set(JSON.parse(readFileSync(ALLOWLIST_PATH, 'utf8')) as string[])
  } catch {
    return new Set()
  }
}

/** Allowlist dışındaki dosyalardaki serbest değerler. */
export function scanRawValues(allow = readAllowlist()): Violation[] {
  return uiFiles()
    .filter((f) => !allow.has(f))
    .flatMap((f) => rawValues(readFileSync(resolve(ROOT, f), 'utf8'), f))
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--write-allowlist')) {
    const legacy = uiFiles().filter((f) => rawValues(readFileSync(resolve(ROOT, f), 'utf8'), f).length > 0)
    writeFileSync(ALLOWLIST_PATH, `${JSON.stringify(legacy.sort(), null, 2)}\n`)
    console.log(`${legacy.length} eski dosya listelendi: ${ALLOWLIST_PATH}`)
  } else {
    const bad = [...scanThemeCopies(), ...scanRawValues()]
    for (const v of bad) console.error(`${v.file}:${v.line}: ${v.value}`)
    if (bad.length) process.exit(1)
    console.log('Tasarım denetimi temiz.')
  }
}
