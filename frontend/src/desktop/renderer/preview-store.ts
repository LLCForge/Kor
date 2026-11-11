// [cdx] Tarayıcı companion seçimleri: önizlemede tıklanan öğeler mesaj kutusunda çip olarak görünür ve bir sonraki
// mesajın başına biçimli eklenir (inceleme yorumlarıyla aynı yaklaşım). Saf kısımlar test edilebilir.
import { useSyncExternalStore } from 'react'
import type { PreviewSelection } from '../ipc-preview.ts'

let selections: readonly PreviewSelection[] = []
const listeners = new Set<() => void>()
const emit = () => { for (const l of listeners) l() }

export function addPreviewSelection(s: PreviewSelection): void {
  if (selections.some((x) => x.selector === s.selector && x.url === s.url)) return
  selections = [...selections, s].slice(-12)
  emit()
}
export function removePreviewSelection(id: string): void {
  selections = selections.filter((s) => s.id !== id)
  emit()
}
export function clearPreviewSelections(): void {
  if (selections.length === 0) return
  selections = []
  emit()
}
export function getPreviewSelections(): readonly PreviewSelection[] {
  return selections
}
export function usePreviewSelections(): readonly PreviewSelection[] {
  return useSyncExternalStore(
    (l) => { listeners.add(l); return () => listeners.delete(l) },
    () => selections,
  )
}

export function selectionLabel(s: PreviewSelection): string {
  const base = s.componentName ? `<${s.componentName}>` : `<${s.tag}>`
  return s.source ? `${base} ${s.source.file.split(/[\\/]/).pop()}:${s.source.line}` : s.text ? `${base} “${s.text.slice(0, 24)}${s.text.length > 24 ? '…' : ''}”` : base
}

/** Mesajın başına eklenecek biçim. */
export function formatPreviewSelections(list: readonly PreviewSelection[]): string {
  if (list.length === 0) return ''
  const lines = list.map((s) => {
    const parts = [`- <${s.tag}> \`${s.selector}\``]
    if (s.componentName) parts.push(`bileşen: ${s.componentName}`)
    if (s.source) parts.push(`kaynak: ${s.source.file}:${s.source.line}${s.source.column != null ? `:${s.source.column}` : ''}`)
    if (s.text) parts.push(`metin: "${s.text.slice(0, 120)}"`)
    parts.push(`sayfa: ${s.url}`)
    return parts.join('\n  ')
  })
  return `Önizlemede seçilen öğeler (${list.length}):\n${lines.join('\n')}`
}

export function mergePreviewIntoMessage(text: string, list: readonly PreviewSelection[]): string {
  const block = formatPreviewSelections(list)
  if (!block) return text
  return text.trim() ? `${block}\n\n${text}` : block
}

export function _resetPreviewSelections(): void {
  selections = []
  emit()
}
