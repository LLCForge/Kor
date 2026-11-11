// parity.toml → renderer/parity-commands.gen.ts (komut paletinin "Tüm komutlar" sayfası, ADR 0031) ve
// renderer tarafı parite testlerinin ortak tarayıcısı. `node scripts/parity-commands.ts` dosyayı yeniden yazar;
// build-ui.ts her derlemede çağırır, renderer/parity.test.ts eşzamanlılığı denetler.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const FRONTEND = resolve(fileURLToPath(import.meta.url), '../..')
export const PARITY_PATH = resolve(FRONTEND, '../parity.toml')
export const GEN_PATH = resolve(FRONTEND, 'src/desktop/renderer/parity-commands.gen.ts')
export const DESKTOP_SRC = resolve(FRONTEND, 'src/desktop')

export interface ParityFeature {
  name: string
  cli: string
  /** "ns.method", kök yöntem ("send") ya da "-" */
  desktop: string
}

// shortcut: parity.toml yalnızca `[[feature]]` + `anahtar = "değer"` satırları kullanıyor (kaçışlı tırnak yok);
// biçim zenginleşirse gerçek bir TOML ayrıştırıcısına geçin.
export function parseParity(text: string): ParityFeature[] {
  const out: ParityFeature[] = []
  for (const block of text.split(/^\[\[feature\]\]\s*$/m).slice(1)) {
    const kv = new Map<string, string>()
    for (const m of block.matchAll(/^(\w+)\s*=\s*(?:"([^"]*)"|(\w+))\s*$/gm)) kv.set(m[1]!, m[2] ?? m[3]!)
    if (kv.get('internal') === 'true') continue
    const name = kv.get('name')
    const desktop = kv.get('desktop')
    if (!name || !desktop) continue
    out.push({ name, cli: kv.get('cli') ?? '', desktop })
  }
  return out
}

/** Paletin ulaştırdığı köprü yöntemleri: `ns.method` biçimindekiler, (ad, yöntem) çiftine göre tekil. */
export function bridgeFeatures(features: readonly ParityFeature[]): ParityFeature[] {
  const seen = new Set<string>()
  return features.filter((f) => {
    const key = `${f.name}\u0000${f.desktop}`
    if (!/^\w+\.\w+$/.test(f.desktop) || seen.has(key)) return false
    seen.add(key)
    return true
  })
}

export function renderGenerated(features: readonly ParityFeature[]): string {
  const rows = bridgeFeatures(features).map((f) => `  { name: ${JSON.stringify(f.name)}, cli: ${JSON.stringify(f.cli)}, method: ${JSON.stringify(f.desktop)} },`)
  return [
    '// ÜRETİLDİ: scripts/parity-commands.ts, kaynak parity.toml. Elle düzenlemeyin (`node scripts/parity-commands.ts`).',
    'export interface ParityCommand {',
    '  /** parity.toml `name` (Türkçe kaynak metin) */',
    '  readonly name: string',
    '  readonly cli: string',
    '  /** Köprü yöntemi `ns.method` */',
    '  readonly method: string',
    '}',
    '',
    'export const PARITY_COMMANDS: readonly ParityCommand[] = [',
    ...rows,
    ']',
    '',
  ].join('\n')
}

export function readParity(): ParityFeature[] {
  return parseParity(readFileSync(PARITY_PATH, 'utf8'))
}

export function writeGenerated(): void {
  const next = renderGenerated(readParity())
  let current = ''
  try { current = readFileSync(GEN_PATH, 'utf8') } catch { /* ilk üretim */ }
  if (current !== next) writeFileSync(GEN_PATH, next)
}

/** `dir` altındaki tüm .ts/.tsx kaynakları (testler ve üretilmiş dosya hariç). */
export function sourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name)
    if (e.isDirectory()) { if (e.name !== 'node_modules') out.push(...sourceFiles(p)) }
    else if (/\.tsx?$/.test(e.name) && !/\.(test|gen|d)\.tsx?$/.test(e.name)) out.push(p)
  }
  return out
}

/**
 * Bir dosyadaki köprü çağrıları (`ns.method` ya da kök `method`): `api.ns.m`, `window.kor.ns.m`, tipli
 * sarmalayıcı (`const xApi = (api as unknown as { ns: X }).ns` → `xApi.m`). `aliases` tüm dosyalardan toplanır
 * (sarmalayıcı başka dosyada dışa aktarılabilir).
 */
export function bridgeCalls(src: string, aliases: ReadonlyMap<string, string>): Set<string> {
  const out = new Set<string>()
  // Zincir satır kırabilir (`api.core\n  .setPrivacy(x)`), noktaların çevresinde boşluk serbest.
  // `(?<![\w/.$])`: içe aktarma yolu ('../api.ts') ve `x.api` eşleşmesin.
  for (const m of src.matchAll(/(?<![\w/.$])(?:api|window\.kor)\s*\.\s*(\w+)(?:\s*\.\s*(\w+))?/g)) out.add(m[2] ? `${m[1]}.${m[2]}` : m[1]!)
  for (const [alias, ns] of aliases) {
    // `-`: an alias named like a file stem (`analysis`) must not match `ipc-analysis.ts`.
    for (const m of src.matchAll(new RegExp(`(?<![\\w/.$-])${alias}\\s*\\.\\s*(\\w+)`, 'g'))) out.add(`${ns}.${m[1]}`)
  }
  return out
}

/**
 * §16.3 (en iyi çaba, statik): `src` içindeki her `ns.method` komut çağrısı için `declared` (dosyanın kendisi ya da
 * panel klasörünün tamamı: düğme çocuk bileşende, çağrı üst bileşende olabilir) `data-kor-cmd` ile o yöntemi
 * bildirmeli; `ns.onX` dinleyicileri için `data-kor-event="kor:ns/..."` olmalı. Eksikleri döndürür.
 */
export function missingKorAttrs(src: string, aliases: ReadonlyMap<string, string>, declared = src): string[] {
  const cmds = korCmdAttrs(declared)
  const missing: string[] = []
  for (const call of bridgeCalls(src, aliases)) {
    const [ns, method] = call.split('.')
    if (!method) continue // kök yöntemler (send, openSession…) sohbet çekirdeğinin işi, panel değil
    if (/^on[A-Z]/.test(method)) {
      if (!new RegExp(`data-kor-event=["'{\`]*kor:${ns}/`).test(declared)) missing.push(`data-kor-event="kor:${ns}/…" (${call})`)
    } else if (!cmds.has(call)) missing.push(`data-kor-cmd="${call}"`)
  }
  return missing
}

/**
 * Tipli sarmalayıcılar: `const x = (api as unknown as { ns: X }).ns` ya da iki adımda
 * `const typed = api as unknown as {…}; export const x = typed.ns`. Sonuç: takma ad → ad alanı.
 */
export function wrapperAliases(src: string): Map<string, string> {
  const out = new Map<string, string>()
  const cast = String.raw`\(?\s*(?:api|window\.kor)\s+as\s+unknown\s+as\s+\{[^}]*\}\s*\)?`
  for (const m of src.matchAll(new RegExp(String.raw`(?:const|let)\s+(\w+)\s*=\s*${cast}(?:\.(\w+))?`, 'g'))) {
    if (m[2]) { out.set(m[1]!, m[2]); continue }
    // Ara nesne (`typed`) dosyaya özeldir; yalnızca ondan türeyen takma adlar dışarı çıkar.
    for (const d of src.matchAll(new RegExp(String.raw`(?:const|let)\s+(\w+)\s*=\s*${m[1]}\.(\w+)\b`, 'g'))) out.set(d[1]!, d[2]!)
  }
  return out
}

/** `data-kor-cmd="a.b c.d"` / `data-kor-cmd={'a.b'}` değerleri. */
export function korCmdAttrs(src: string): Set<string> {
  const out = new Set<string>()
  for (const m of src.matchAll(/data-kor-cmd=(?:"([^"]*)"|\{\s*['"`]([^'"`]*)['"`]\s*\})/g)) {
    for (const v of (m[1] ?? m[2] ?? '').split(/\s+/)) if (v) out.add(v)
  }
  return out
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) writeGenerated()
