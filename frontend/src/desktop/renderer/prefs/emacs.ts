// Mesaj kutusu için emacs tuş düzeni (§28.10, ADR 0050). Saf; node --test ile sınanır.
// C-a/C-e satır başı/sonu, C-b/C-f karakter, M-b/M-f sözcük, C-p/C-n satır yukarı/aşağı, C-k satır sonuna
// kadar kes, C-d karakter sil, C-h geri sil, C-w / M-Backspace önceki sözcüğü kes, M-d sonraki sözcüğü kes,
// C-y son kesileni yapıştır.

export interface EmacsState {
  /** Son kesilen metin (C-y) */
  kill: string
}

export interface KeyInput {
  key: string
  ctrl: boolean
  alt: boolean
}

export interface EmacsResult {
  state: EmacsState
  text: string
  cursor: number
  /** true: tuş emacs tarafından işlendi, varsayılan davranış engellenmeli */
  handled: boolean
}

export const initialEmacsState: EmacsState = { kill: '' }

const lineStart = (t: string, p: number) => t.lastIndexOf('\n', p - 1) + 1
function lineEnd(t: string, p: number): number {
  const e = t.indexOf('\n', p)
  return e === -1 ? t.length : e
}
const isWord = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}_]/u.test(c)

function wordForward(t: string, p: number): number {
  let i = p
  while (i < t.length && !isWord(t[i])) i++
  while (i < t.length && isWord(t[i])) i++
  return i
}

function wordBack(t: string, p: number): number {
  let i = p
  while (i > 0 && !isWord(t[i - 1])) i--
  while (i > 0 && isWord(t[i - 1])) i--
  return i
}

function vertical(t: string, p: number, dir: 1 | -1): number {
  const start = lineStart(t, p)
  const col = p - start
  if (dir === 1) {
    const end = lineEnd(t, p)
    if (end >= t.length) return p
    const next = end + 1
    return Math.min(next + col, lineEnd(t, next))
  }
  if (start === 0) return p
  const prev = lineStart(t, start - 1)
  return Math.min(prev + col, start - 1)
}

export function emacsKey(state: EmacsState, k: KeyInput, text: string, cursor: number): EmacsResult {
  const move = (c: number): EmacsResult => ({ state, text, cursor: c, handled: true })
  const cut = (from: number, to: number): EmacsResult => ({
    state: { kill: text.slice(from, to) },
    text: text.slice(0, from) + text.slice(to),
    cursor: from,
    handled: true,
  })
  const none: EmacsResult = { state, text, cursor, handled: false }
  if (k.ctrl && !k.alt) {
    switch (k.key.toLowerCase()) {
      case 'a':
        return move(lineStart(text, cursor))
      case 'e':
        return move(lineEnd(text, cursor))
      case 'b':
        return move(Math.max(0, cursor - 1))
      case 'f':
        return move(Math.min(text.length, cursor + 1))
      case 'p':
        return move(vertical(text, cursor, -1))
      case 'n':
        return move(vertical(text, cursor, 1))
      case 'k': {
        const end = lineEnd(text, cursor)
        // Satır sonundaysa satır ayırıcısını keser (emacs gibi).
        return cut(cursor, end === cursor ? Math.min(text.length, end + 1) : end)
      }
      case 'd':
        return cursor < text.length ? { state, text: text.slice(0, cursor) + text.slice(cursor + 1), cursor, handled: true } : move(cursor)
      case 'h':
        return cursor > 0 ? { state, text: text.slice(0, cursor - 1) + text.slice(cursor), cursor: cursor - 1, handled: true } : move(cursor)
      case 'w':
        return cut(wordBack(text, cursor), cursor)
      case 'y':
        return { state, text: text.slice(0, cursor) + state.kill + text.slice(cursor), cursor: cursor + state.kill.length, handled: true }
      default:
        return none
    }
  }
  if (k.alt && !k.ctrl) {
    switch (k.key.toLowerCase()) {
      case 'b':
        return move(wordBack(text, cursor))
      case 'f':
        return move(wordForward(text, cursor))
      case 'd':
        return cut(cursor, wordForward(text, cursor))
      case 'backspace':
        return cut(wordBack(text, cursor), cursor)
      default:
        return none
    }
  }
  return none
}
