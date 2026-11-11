// Dosya tipi ikonu (cdesktop dosya kartı/ağacı): önce gerçek SVG ikon (material-icon-theme, MIT), yoksa renkli metin rozeti.
import type { CSSProperties } from 'react'
import cSvg from './file-icons/c.svg'
import consoleSvg from './file-icons/console.svg'
import cppSvg from './file-icons/cpp.svg'
import csharpSvg from './file-icons/csharp.svg'
import cssSvg from './file-icons/css.svg'
import databaseSvg from './file-icons/database.svg'
import dockerSvg from './file-icons/docker.svg'
import documentSvg from './file-icons/document.svg'
import gitSvg from './file-icons/git.svg'
import goSvg from './file-icons/go.svg'
import htmlSvg from './file-icons/html.svg'
import imageSvg from './file-icons/image.svg'
import javaSvg from './file-icons/java.svg'
import javascriptSvg from './file-icons/javascript.svg'
import jsonSvg from './file-icons/json.svg'
import kotlinSvg from './file-icons/kotlin.svg'
import lockSvg from './file-icons/lock.svg'
import markdownSvg from './file-icons/markdown.svg'
import pdfSvg from './file-icons/pdf.svg'
import powershellSvg from './file-icons/powershell.svg'
import pythonSvg from './file-icons/python.svg'
import reactSvg from './file-icons/react.svg'
import reactTsSvg from './file-icons/react_ts.svg'
import rustSvg from './file-icons/rust.svg'
import sassSvg from './file-icons/sass.svg'
import svgSvg from './file-icons/svg.svg'
import tomlSvg from './file-icons/toml.svg'
import typescriptSvg from './file-icons/typescript.svg'
import xmlSvg from './file-icons/xml.svg'
import yamlSvg from './file-icons/yaml.svg'
import zipSvg from './file-icons/zip.svg'

/** Uzantı → SVG ikon. Tam dosya adları (Dockerfile, .gitignore) da eşlenir. */
const ICON_BY_EXT: Record<string, string> = {
  ts: typescriptSvg, mts: typescriptSvg, cts: typescriptSvg, tsx: reactTsSvg,
  js: javascriptSvg, mjs: javascriptSvg, cjs: javascriptSvg, jsx: reactSvg,
  py: pythonSvg, pyi: pythonSvg, rs: rustSvg, go: goSvg, java: javaSvg, kt: kotlinSvg, kts: kotlinSvg, cs: csharpSvg,
  cpp: cppSvg, cc: cppSvg, cxx: cppSvg, hpp: cppSvg, hh: cppSvg, c: cSvg, h: cSvg,
  css: cssSvg, scss: sassSvg, sass: sassSvg, html: htmlSvg, htm: htmlSvg, md: markdownSvg, mdx: markdownSvg,
  json: jsonSvg, jsonc: jsonSvg, json5: jsonSvg, yaml: yamlSvg, yml: yamlSvg, toml: tomlSvg,
  sql: databaseSvg, db: databaseSvg, sqlite: databaseSvg,
  sh: consoleSvg, bash: consoleSvg, zsh: consoleSvg, bat: consoleSvg, cmd: consoleSvg, ps1: powershellSvg,
  svg: svgSvg, xml: xmlSvg, lock: lockSvg,
  png: imageSvg, jpg: imageSvg, jpeg: imageSvg, gif: imageSvg, webp: imageSvg, ico: imageSvg, bmp: imageSvg,
  pdf: pdfSvg, zip: zipSvg, gz: zipSvg, tgz: zipSvg, tar: zipSvg, rar: zipSvg, '7z': zipSvg,
}
const ICON_BY_NAME: Record<string, string> = {
  dockerfile: dockerSvg, '.gitignore': gitSvg, '.gitattributes': gitSvg, '.gitmodules': gitSvg,
  'package-lock.json': lockSvg, 'yarn.lock': lockSvg, 'pnpm-lock.yaml': lockSvg, 'cargo.lock': lockSvg,
}

/** Metin rozeti yedeği: SVG'si olmayan uzantılar (ve ikon kapalıyken) için etiket + marka rengi. */
const LANG_BADGES: Record<string, { label: string; bg: string; fg?: string }> = {
  ts: { label: 'TS', bg: '#3178c6' }, tsx: { label: 'TS', bg: '#3178c6' }, mts: { label: 'TS', bg: '#3178c6' },
  js: { label: 'JS', bg: '#f7df1e', fg: '#222' }, jsx: { label: 'JS', bg: '#f7df1e', fg: '#222' }, mjs: { label: 'JS', bg: '#f7df1e', fg: '#222' }, cjs: { label: 'JS', bg: '#f7df1e', fg: '#222' },
  py: { label: 'PY', bg: '#3572a5' }, rs: { label: 'RS', bg: '#dea584', fg: '#222' }, go: { label: 'GO', bg: '#00add8' }, java: { label: 'JV', bg: '#b07219' },
  kt: { label: 'KT', bg: '#7f52ff' }, cs: { label: 'C#', bg: '#178600' }, cpp: { label: 'C++', bg: '#f34b7d' }, c: { label: 'C', bg: '#555' },
  css: { label: 'CSS', bg: '#563d7c' }, scss: { label: 'SCSS', bg: '#c6538c' }, html: { label: '<>', bg: '#e34c26' }, md: { label: 'MD', bg: '#083fa1' }, mdx: { label: 'MDX', bg: '#083fa1' },
  json: { label: '{ }', bg: '#444' }, yaml: { label: 'YML', bg: '#cb171e' }, yml: { label: 'YML', bg: '#cb171e' }, toml: { label: 'TOML', bg: '#9c4221' },
  sql: { label: 'SQL', bg: '#e38c00', fg: '#222' }, sh: { label: '>_', bg: '#89e051', fg: '#222' }, ps1: { label: 'PS', bg: '#012456' }, bat: { label: 'BAT', bg: '#444' }, svg: { label: 'SVG', bg: '#ffb13b', fg: '#222' },
  txt: { label: 'TXT', bg: '#666' }, env: { label: 'ENV', bg: '#e0b400', fg: '#222' }, csv: { label: 'CSV', bg: '#1d6f42' },
}

function fileParts(path: string): { name: string; ext: string } {
  const name = (path.split(/[\\/]/).pop() ?? '').toLowerCase()
  const dot = name.lastIndexOf('.')
  return { name, ext: dot >= 0 ? name.slice(dot + 1) : '' }
}

/** SVG ikon metni; hiçbiri uymuyorsa null (metin rozeti ya da genel belge ikonu kullanılır). */
export function fileIconFor(path: string): string | null {
  const { name, ext } = fileParts(path)
  return ICON_BY_NAME[name] ?? ICON_BY_EXT[ext] ?? null
}

export function langBadgeFor(path: string): { label: string; bg: string; fg?: string } | null {
  return LANG_BADGES[fileParts(path).ext] ?? null
}

/**
 * Dosya ikonu: SVG varsa o; yoksa metin rozeti; o da yoksa genel belge ikonu (generic=true ise).
 * size='sm' ağaç satırları için.
 */
export function LangBadge({ path, size = 'md', generic = false }: { path: string; size?: 'sm' | 'md'; generic?: boolean }) {
  const cls = `lang-badge${size === 'sm' ? ' sm' : ''}`
  const svg = fileIconFor(path)
  if (svg) return <span className={`${cls} icon`} aria-hidden="true" dangerouslySetInnerHTML={{ __html: svg }} />
  const b = langBadgeFor(path)
  if (b) {
    const style: CSSProperties = { background: b.bg, color: b.fg ?? '#fff' }
    return (
      <span className={cls} style={style} aria-hidden="true">
        {b.label}
      </span>
    )
  }
  if (generic) return <span className={`${cls} icon`} aria-hidden="true" dangerouslySetInnerHTML={{ __html: documentSvg }} />
  return null
}
