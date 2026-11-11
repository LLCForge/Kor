// Büyük yapıştırmaların katlanması (saf; test edilebilir).

export const PASTE_FOLD_LINES = 40
export const PASTE_FOLD_CHARS = 4000

export const lineCount = (text: string) => text.replace(/\r?\n$/, '').split(/\r?\n/).length

export function shouldFold(text: string): boolean {
  return text.length > PASTE_FOLD_CHARS || lineCount(text) > PASTE_FOLD_LINES
}

/** Taslakta görünen yer tutucu: "[Yapıştırılan 120 satır #1]" */
export function pasteLabel(text: string, id: number): string {
  const lines = lineCount(text)
  return lines > 1 ? `[Yapıştırılan ${lines} satır #${id}]` : `[Yapıştırılan ${text.length} karakter #${id}]`
}

const TOKEN = /\[Yapıştırılan \d+ (?:satır|karakter) #(\d+)\]/g

/** Taslakta hâlâ duran yapıştırma kimlikleri (silinenler gönderilmez). */
export function pastesInDraft(draft: string, pastes: ReadonlyMap<number, string>): number[] {
  return [...draft.matchAll(TOKEN)].map((m) => Number(m[1])).filter((id) => pastes.has(id))
}

/** Gönderimde yer tutucular tam metne açılır. */
export function expandPastes(draft: string, pastes: ReadonlyMap<number, string>): string {
  return draft.replace(TOKEN, (token, id: string) => pastes.get(Number(id)) ?? token)
}

/** Tek bir yer tutucuyu taslakta açar (çipe tıklanınca). */
export function expandPaste(draft: string, pastes: ReadonlyMap<number, string>, id: number): string {
  return draft.replace(TOKEN, (token, tokenId: string) => (Number(tokenId) === id ? (pastes.get(id) ?? token) : token))
}
