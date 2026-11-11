// Okunabilirlik ayarları (§28.9, ADR 0050): `ui.*` değerlerinden kök öğe sınıfları ve CSS değişkenleri.
// Saf işlevler (node --test ile sınanır); uygulanması PrefsLayer'da.
import type { DiffViewMode, LineWidth, ToolOutput, UiPrefs } from '../../ipc-prefs.ts'

export const DEFAULT_UI: UiPrefs = {
  wrap: true,
  wrapWidth: 0,
  maxWidth: 'medium',
  density: 'comfortable',
  fontSize: 14,
  lineHeight: 1.5,
  codeFont: '',
  diffView: 'inline',
  toolOutput: 'normal',
  screenReader: false,
  highContrast: false,
  reducedMotion: false,
  keys: 'default',
}

/** Satır genişliği → `--chat-width` (mevcut tema değişkeni). */
export const CHAT_WIDTH: Record<LineWidth, string> = {
  narrow: 'min(640px, 100%)',
  medium: 'clamp(768px, 72vw, 1120px)',
  wide: 'min(1400px, 92vw)',
  full: '100%',
}

/** Yazı tipi adı: yalnızca harf, rakam, boşluk ve tire (CSS'e enjeksiyon olmasın). */
export function safeFontName(name: string): string | null {
  const n = name.trim()
  return n && n.length <= 60 && /^[\p{L}\p{N} _-]+$/u.test(n) ? n : null
}

export function cssVars(u: UiPrefs): Record<string, string> {
  const font = safeFontName(u.codeFont)
  return {
    '--chat-width': CHAT_WIDTH[u.maxWidth],
    '--kor-chat-font-size': `${Math.min(24, Math.max(12, Math.round(u.fontSize)))}px`,
    '--kor-chat-line-height': String(Math.min(2.2, Math.max(1.1, u.lineHeight))),
    '--kor-code-font': font ? `'${font}', var(--font-mono)` : 'var(--font-mono)',
  }
}

export function rootClasses(u: UiPrefs): string[] {
  const out = ['kor-ui']
  if (!u.wrap) out.push('kor-ui-nowrap')
  if (u.density === 'compact') out.push('kor-ui-compact')
  if (u.reducedMotion || u.screenReader) out.push('kor-ui-still')
  if (u.highContrast) out.push('kor-ui-contrast')
  if (u.screenReader) out.push('kor-ui-reader')
  if (u.toolOutput === 'quiet') out.push('kor-ui-quiet')
  if (u.toolOutput === 'verbose') out.push('kor-ui-verbose')
  return out
}

/** Yapılandırmadaki diff görünümü → DiffView kipleri. */
export const diffMode = (v: DiffViewMode): 'unified' | 'split' => (v === 'split' ? 'split' : 'unified')

let current: UiPrefs = DEFAULT_UI
export const currentUi = (): UiPrefs => current
export function setCurrentUi(u: UiPrefs): void {
  current = u
}
/** Araç kartı varsayılan olarak açık mı (`ui.tool_output = verbose`). */
export const toolsOpenByDefault = (): boolean => current.toolOutput === ('verbose' satisfies ToolOutput)

let soundAllowed = true
/** `[auto] sound` (§28.14): bildirim sesi kendiliğinden çalınsın mı (PrefsLayer yükler). */
export const autoSoundOn = (): boolean => soundAllowed
export function setAutoSound(on: boolean): void {
  soundAllowed = on
}
