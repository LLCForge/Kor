// [g21] Belgeden alıntıyı sohbete taşıma (#187). Önizleme, Markdown metnini bir pencere olayıyla yayınlar; App
//      bunu mesaj kutusu taslağının sonuna ekler. Metin yalnızca taslağa girer, otomatik GÖNDERİLMEZ.
import { useEffect, type Dispatch, type SetStateAction } from 'react'

export const DRAFT_EVENT = 'g21:draft-insert'
export const MAX_QUOTE_CHARS = 4_000
const MAX_INSERT_CHARS = 64_000

export interface QuoteDetail {
  text: string
  /** Kaynak etiketi (ör. "docs/rapor.pdf, s. 3") */
  source: string
}

/** Seçili metni Markdown alıntı bloğuna çevirir (satır başları "> ", sonda kaynak satırı). */
export function formatQuote({ text, source }: QuoteDetail): string {
  const clean = text.replace(/\r\n?/g, '\n').replace(/\n{3,}/g, '\n\n').trim().slice(0, MAX_QUOTE_CHARS)
  if (!clean) return ''
  const body = clean
    .split('\n')
    .map((line) => (line.trim() ? `> ${line.trimEnd()}` : '>'))
    .join('\n')
  const label = source.replace(/[\r\n]+/g, ' ').trim()
  return label ? `${body}\n>\n> (kaynak: ${label})\n` : `${body}\n`
}

/** Mevcut taslağın sonuna metni ekler (araya boş satır koyarak). */
export function appendToDraft(draft: string, text: string): string {
  if (!text) return draft
  if (!draft.trim()) return text
  return `${draft.replace(/\s+$/, '')}\n\n${text}`
}

/** Hazır Markdown metnini mesaj kutusu taslağına gönderir. */
export function insertIntoDraft(markdown: string): void {
  if (markdown) window.dispatchEvent(new CustomEvent<string>(DRAFT_EVENT, { detail: markdown }))
}

/** Seçili belge metnini alıntı olarak taslağa ekler. */
export function quoteToDraft(detail: QuoteDetail): void {
  insertIntoDraft(formatQuote(detail))
}

/** App: taslağa ekleme olaylarını dinler. */
export function useG21DraftInsert(setDraft: Dispatch<SetStateAction<string>>): void {
  useEffect(() => {
    const onInsert = (e: Event) => {
      const text: unknown = (e as CustomEvent<unknown>).detail
      if (typeof text === 'string' && text) setDraft((d) => appendToDraft(d, text.slice(0, MAX_INSERT_CHARS)))
    }
    window.addEventListener(DRAFT_EVENT, onInsert)
    return () => window.removeEventListener(DRAFT_EVENT, onInsert)
  }, [setDraft])
}
