// assets/providers/<id>.svg (+ <id>-color.svg) → renderer/provider-logos.gen.ts (spec §23.2, ADR 0053).
// Logolar tek yerde (assets/providers) yaşar; masaüstü bu üretilen içe aktarma listesini kullanır.
// `node scripts/provider-logos.ts` yazar (değişmediyse dokunmaz); build-ui.ts her derlemede çağırır.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const FRONTEND = resolve(fileURLToPath(import.meta.url), '../..')
export const LOGO_DIR = resolve(FRONTEND, '../assets/providers')
export const GEN = resolve(FRONTEND, 'src/desktop/renderer/provider-logos.gen.ts')

export interface LogoFile {
  id: string
  color: boolean
}

/** Klasördeki logolar: her kimlik için tek renk dosyası zorunlu, renkli varyant isteğe bağlı. */
export function scanLogos(files: readonly string[]): LogoFile[] {
  const ids = files.filter((f) => /^[a-z0-9]+\.svg$/.test(f)).map((f) => f.slice(0, -4))
  return ids.sort().map((id) => ({ id, color: files.includes(`${id}-color.svg`) }))
}

const ident = (id: string, color: boolean) => `${id.replace(/[^a-z0-9]/g, '_')}${color ? '_color' : ''}`

export function renderGen(logos: readonly LogoFile[]): string {
  const imports = logos.flatMap((l) => [
    `import ${ident(l.id, false)} from '../../../../assets/providers/${l.id}.svg'`,
    ...(l.color ? [`import ${ident(l.id, true)} from '../../../../assets/providers/${l.id}-color.svg'`] : []),
  ])
  const rows = logos.map((l) => `  ${JSON.stringify(l.id)}: { mono: ${ident(l.id, false)}, color: ${l.color ? ident(l.id, true) : ident(l.id, false)} },`)
  return [
    '// ÜRETİLDİ: scripts/provider-logos.ts, kaynak assets/providers/*.svg. Elle düzenlemeyin.',
    ...imports,
    '',
    '/** Logo kimliği → tek renk (currentColor) ve tam renkli SVG metni (renkli varyant yoksa tek renk). */',
    'export const PROVIDER_LOGOS: Readonly<Record<string, { mono: string; color: string }>> = {',
    ...rows,
    '}',
    '',
  ].join('\n')
}

export function writeGenerated(): boolean {
  const text = renderGen(scanLogos(readdirSync(LOGO_DIR)))
  let cur = ''
  try {
    cur = readFileSync(GEN, 'utf8').replace(/\r\n/g, '\n')
  } catch {
    // ilk üretim
  }
  if (cur === text) return false
  writeFileSync(GEN, text)
  return true
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(writeGenerated() ? `Üretildi: ${GEN}` : 'Logolar güncel.')
}
