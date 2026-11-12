// assets/theme.json (+ assets/verbs.json) → üretilen tema dosyaları (spec §16.1, §16.2 kural 1, §23.1, ADR 0053):
//   assets/kor-theme.ts                                 (TS sabitleri: banner.ts / kor-spinner.ts referansı)
//   frontend/src/desktop/renderer/styles/theme.gen.css   (masaüstü CSS değişkenleri)
//   frontend/extension/theme.css                         (Chrome yan paneli, aynı değişkenler)
//   backend/crates/kor-core/src/theme.rs                (CLI/TUI Rust sabitleri; kendi testi eşzamanlılığı denetler)
//   frontend/src-tauri/tauri.conf.json                  (yalnızca ana pencerenin backgroundColor alanı)
// `node scripts/theme-gen.ts` yazar (değişmeyene dokunmaz); `--check` yazmaz, fark varsa 1 ile çıkar.
// build-ui.ts her derlemede çağırır.
import { spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const FRONTEND = resolve(fileURLToPath(import.meta.url), '../..')
export const ROOT = resolve(FRONTEND, '..')
export const THEME_PATH = resolve(ROOT, 'assets/theme.json')
export const VERBS_PATH = resolve(ROOT, 'assets/verbs.json')
export const OUT = {
  ts: resolve(ROOT, 'assets/kor-theme.ts'),
  css: resolve(FRONTEND, 'src/desktop/renderer/styles/theme.gen.css'),
  /** Chrome yan paneli aynı token'ları kullanır (§21.2, §23) */
  extCss: resolve(FRONTEND, 'extension/theme.css'),
  rust: resolve(ROOT, 'backend/crates/kor-core/src/theme.rs'),
  tauri: resolve(FRONTEND, 'src-tauri/tauri.conf.json'),
} as const

export interface Theme {
  ember: Record<'hi' | 'mid' | 'lo' | 'deep', string>
  ink: Record<'bg' | 'faceTop' | 'faceLeft' | 'faceRight' | 'cream', string>
  surfaces: Record<'bg' | 'surface' | 'raised' | 'overlay', keyof Theme['ink']>
  border: { color: string; alpha: number }
  status: Record<'success' | 'warning' | 'error' | 'info', string>
  gradient: string[]
  /** Categorical colours for graph communities (project graph, graphify-style view). */
  graph: string[]
  frames: Record<string, string[]>
  ansi256: Record<string, number>
  fonts: Record<'heading' | 'ui' | 'mono', string>
  fontSize: number[]
  lineHeight: number
  space: number[]
  radius: Record<'chip' | 'card' | 'composer', number>
  focusRing: { width: number; alpha: number }
  motion: { hover: number; menu: number; panel: number; easing: string }
  layout: Record<'railCollapsed' | 'chatMax' | 'drawerBelow' | 'pageEdge' | 'cardPad' | 'sectionGap', number>
}
export interface Verbs { tr: string[]; en: string[]; fr: string[] }

const HEX = /^#[0-9A-Fa-f]{6}$/

/** Şemayı doğrular: üretici bozuk bir kaynaktan sessizce yanlış sabit üretmez. */
export function validateTheme(t: Theme): string[] {
  const errors: string[] = []
  const hexes: [string, string][] = [
    ...Object.entries(t.ember).map(([k, v]): [string, string] => [`ember.${k}`, v]),
    ...Object.entries(t.ink).map(([k, v]): [string, string] => [`ink.${k}`, v]),
    ...Object.entries(t.status).map(([k, v]): [string, string] => [`status.${k}`, v]),
    ...t.gradient.map((v, i): [string, string] => [`gradient[${i}]`, v]),
    ...t.graph.map((v, i): [string, string] => [`graph[${i}]`, v]),
    ['border.color', t.border.color],
  ]
  for (const [k, v] of hexes) if (!HEX.test(v)) errors.push(`${k}: "${v}" #RRGGBB değil`)
  for (const [k, v] of Object.entries(t.surfaces)) if (!(v in t.ink)) errors.push(`surfaces.${k}: ink.${v} yok`)
  if (t.gradient.length !== 8) errors.push('gradient 8 adım olmalı')
  if (!t.frames.ember?.length) errors.push('frames.ember boş')
  return errors
}

export const rgbOf = (hex: string): [number, number, number] => {
  const n = Number.parseInt(hex.slice(1), 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}
const kebab = (s: string) => s.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)
const upper = (s: string) => kebab(s).replace(/-/g, '_').toUpperCase()
const rgba = (hex: string, a: number) => `rgb(${rgbOf(hex).join(' ')} / ${a})`

const FALLBACK = {
  heading: "ui-serif, Georgia, Cambria, 'Times New Roman', serif",
  ui: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif",
  mono: "ui-monospace, 'Cascadia Code', Consolas, Menlo, monospace",
} as const

export function renderCss(t: Theme): string {
  const v: string[] = []
  for (const [k, x] of Object.entries(t.ember)) v.push(`--kor-ember-${k}: ${x};`)
  for (const [k, x] of Object.entries(t.ink)) v.push(`--kor-ink-${kebab(k)}: ${x};`)
  for (const [k, x] of Object.entries(t.surfaces)) v.push(`--kor-layer-${k}: ${t.ink[x]};`)
  v.push(`--kor-border: ${rgba(t.border.color, t.border.alpha)};`)
  for (const [k, x] of Object.entries(t.status)) v.push(`--kor-${k}: ${x};`)
  t.gradient.forEach((x, i) => v.push(`--kor-gradient-${i}: ${x};`))
  t.graph.forEach((x, i) => v.push(`--kor-graph-${i}: ${x};`))
  v.push(`--kor-ember-soft: ${rgba(t.ember.mid, t.focusRing.alpha)};`)
  v.push(`--kor-ring: 0 0 0 ${t.focusRing.width}px ${rgba(t.ember.mid, t.focusRing.alpha)};`)
  for (const [k, x] of Object.entries(t.fonts)) v.push(`--kor-font-${k}: '${x}', ${FALLBACK[k as keyof typeof FALLBACK]};`)
  for (const px of t.fontSize) v.push(`--kor-text-${px}: ${px}px;`)
  v.push(`--kor-leading: ${t.lineHeight};`)
  t.space.forEach((px, i) => v.push(`--kor-space-${i + 1}: ${px}px;`))
  for (const [k, px] of Object.entries(t.radius)) v.push(`--kor-radius-${k}: ${px}px;`)
  for (const k of ['hover', 'menu', 'panel'] as const) v.push(`--kor-dur-${k}: ${t.motion[k]}ms;`)
  v.push(`--kor-ease: ${t.motion.easing};`)
  for (const [k, px] of Object.entries(t.layout)) v.push(`--kor-${kebab(k)}: ${px}px;`)
  return [
    '/* ÜRETİLDİ: frontend/scripts/theme-gen.ts, kaynak assets/theme.json. Elle düzenlemeyin. */',
    ':root {',
    ...v.map((l) => `  ${l}`),
    '}',
    // Hareket azaltılınca süreler sıfırlanır (spec §16.2 kural 3, §23.1).
    '@media (prefers-reduced-motion: reduce) {',
    '  :root { --kor-dur-hover: 0ms; --kor-dur-menu: 0ms; --kor-dur-panel: 0ms; }',
    '}',
    '',
  ].join('\n')
}

const rsRgb = (hex: string) => `(${rgbOf(hex).join(', ')})`
const rsStrs = (xs: readonly string[]) => `[${xs.map((x) => JSON.stringify(x)).join(', ')}]`

export function renderRust(t: Theme): string {
  const group = (name: string, doc: string, rec: Record<string, string>) => [
    `/// ${doc}`,
    `pub mod ${name} {`,
    '    use super::Rgb;',
    ...Object.entries(rec).map(([k, x]) => `    pub const ${upper(k)}: Rgb = ${rsRgb(x)};`),
    '}',
    '',
  ]
  return [
    '//! GENERATED by `frontend/scripts/theme-gen.ts` from `assets/theme.json` — do not edit (spec §16.2 rule 1).',
    '//! The test below fails when this file and `assets/theme.json` drift apart.',
    '',
    '/// An sRGB colour.',
    'pub type Rgb = (u8, u8, u8);',
    '',
    ...group('ember', 'Ember accents: primary button, focus ring, active state, spinner, logo.', t.ember),
    ...group('ink', 'Neutral ink surfaces.', t.ink),
    ...group('status', 'Status colours (success, warning, error, info).', t.status),
    '/// The 8-step ember gradient used by the spinner and the banner.',
    `pub const GRADIENT: [Rgb; ${t.gradient.length}] = [${t.gradient.map(rsRgb).join(', ')}];`,
    '',
    '/// Spinner frame sets.',
    'pub mod frames {',
    ...Object.entries(t.frames).map(([k, xs]) => `    pub const ${upper(k)}: [&str; ${xs.length}] = ${rsStrs(xs)};`),
    '}',
    '',
    '/// 256-colour fallbacks for terminals without truecolor.',
    'pub mod ansi256 {',
    ...Object.entries(t.ansi256).map(([k, n]) => `    pub const ${upper(k)}: u8 = ${n};`),
    '}',
    '',
    '#[cfg(test)]',
    'mod tests {',
    '    use super::*;',
    '',
    "    #[allow(clippy::many_single_char_names)] // r, g, b, h, s, n: the colour math's own names",
    '    fn rgb(v: &serde_json::Value) -> Rgb {',
    '        let s = v.as_str().expect("hex string");',
    '        let n = u32::from_str_radix(s.trim_start_matches(\'#\'), 16).expect("hex");',
    '        let [_, r, g, b] = n.to_be_bytes();',
    '        (r, g, b)',
    '    }',
    '',
    '    #[test]',
    '    fn in_sync_with_theme_json() {',
    '        let t: serde_json::Value =',
    '            serde_json::from_str(include_str!("../../../../assets/theme.json")).expect("theme.json");',
    ...Object.keys(t.ember).map((k) => `        assert_eq!(ember::${upper(k)}, rgb(&t["ember"]["${k}"]), "run theme-gen");`),
    ...Object.keys(t.ink).map((k) => `        assert_eq!(ink::${upper(k)}, rgb(&t["ink"]["${k}"]), "run theme-gen");`),
    ...Object.keys(t.status).map((k) => `        assert_eq!(status::${upper(k)}, rgb(&t["status"]["${k}"]), "run theme-gen");`),
    '        let g: Vec<Rgb> = t["gradient"].as_array().expect("gradient").iter().map(rgb).collect();',
    '        assert_eq!(GRADIENT.to_vec(), g, "run theme-gen");',
    '        let ember_frames: Vec<&str> = t["frames"]["ember"].as_array().expect("frames").iter().filter_map(|v| v.as_str()).collect();',
    '        assert_eq!(frames::EMBER.to_vec(), ember_frames, "run theme-gen");',
    '    }',
    '}',
    '',
  ].join('\n')
}

export function renderTs(t: Theme, verbs: Verbs): string {
  const j = (x: unknown) => JSON.stringify(x)
  return [
    '/** ÜRETİLDİ: frontend/scripts/theme-gen.ts, kaynak assets/theme.json + assets/verbs.json. Elle düzenlemeyin. */',
    `export const EMBER = ${j(t.ember)} as const;`,
    `export const INK = ${j(t.ink)} as const;`,
    `export const STATUS = ${j(t.status)} as const;`,
    `export const GRADIENT = ${j(t.gradient)};`,
    `export const FRAMES = ${j(t.frames)} as const;`,
    `export const ANSI256 = ${j(t.ansi256)} as const;`,
    `export const VERBS_TR = ${j(verbs.tr)};`,
    `export const VERBS_EN = ${j(verbs.en)};`,
    `export const VERBS_FR = ${j(verbs.fr)};`,
    '',
  ].join('\n')
}

/** tauri.conf.json'da yalnızca ana pencerenin arka planını günceller; diğer alanlara dokunmaz. */
export function patchTauri(text: string, t: Theme): string {
  const conf = JSON.parse(text) as { app?: { windows?: { backgroundColor?: string }[] } }
  const win = conf.app?.windows?.[0]
  if (!win || win.backgroundColor === t.ink.bg) return text
  win.backgroundColor = t.ink.bg
  return `${JSON.stringify(conf, null, 2)}\n`
}

export function loadTheme(): { theme: Theme; verbs: Verbs } {
  const theme = JSON.parse(readFileSync(THEME_PATH, 'utf8')) as Theme
  const verbs = JSON.parse(readFileSync(VERBS_PATH, 'utf8')) as Verbs
  const errors = validateTheme(theme)
  if (errors.length) throw new Error(`assets/theme.json geçersiz:\n${errors.join('\n')}`)
  return { theme, verbs }
}

/** Her çıktının beklenen içeriği (yol → metin). */
export function expectedOutputs(): Map<string, string> {
  const { theme, verbs } = loadTheme()
  const tauri = readFileSync(OUT.tauri, 'utf8')
  return new Map([
    [OUT.ts, renderTs(theme, verbs)],
    [OUT.css, renderCss(theme)],
    [OUT.extCss, renderCss(theme)],
    [OUT.rust, renderRust(theme)],
    [OUT.tauri, patchTauri(tauri, theme)],
  ])
}

const read = (p: string) => {
  try {
    return readFileSync(p, 'utf8').replace(/\r\n/g, '\n')
  } catch {
    return null
  }
}

/** Rust çıktısı rustfmt'ten geçer; bu yüzden .rs karşılaştırması boşluktan bağımsızdır. */
function same(path: string, text: string): boolean {
  const cur = read(path)
  if (cur === null) return false
  // rustfmt yalnızca boşluk ve sondaki virgülleri değiştirir.
  const norm = (s: string) => s.replace(/\s+/g, '').replace(/,([\])}])/g, '$1')
  if (path.endsWith('.rs')) return norm(cur) === norm(text)
  return cur === text.replace(/\r\n/g, '\n')
}

/** Değişen dosyaları yazar; yazılanların listesini döner. */
export function writeGenerated(): string[] {
  const written: string[] = []
  for (const [path, text] of expectedOutputs()) {
    if (same(path, text)) continue
    writeFileSync(path, text)
    // rustfmt yoksa (yalnızca Node kurulu) biçimsiz ama geçerli Rust kalır; Rust testi yine eşzamanlılığı denetler.
    if (path.endsWith('.rs')) spawnSync('rustfmt', ['--edition', '2024', path], { stdio: 'ignore' })
    written.push(path)
  }
  return written
}

/** Eşzamanlı olmayan çıktıların listesi (boşsa her şey güncel). */
export function staleOutputs(): string[] {
  return [...expectedOutputs()].filter(([p, text]) => !same(p, text)).map(([p]) => p)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--check')) {
    const stale = staleOutputs()
    if (stale.length) {
      console.error(`Tema çıktıları eski, \`node scripts/theme-gen.ts\` çalıştırın:\n${stale.join('\n')}`)
      process.exit(1)
    }
  } else {
    const w = writeGenerated()
    console.log(w.length ? `Tema üretildi:\n${w.join('\n')}` : 'Tema güncel.')
  }
}
