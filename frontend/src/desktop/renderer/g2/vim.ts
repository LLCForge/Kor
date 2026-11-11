// Mesaj kutusu için temel vim alt kümesi (saf; test edilebilir).
// Normal kip: h j k l w b 0 $ x dd p; ekleme kipine geçiş: i a o A I; Esc ekleme kipinden çıkar.
// Görsel kip (§28.10, ADR 0050): v seçimi başlatır, hareketler seçimi uzatır, d/x keser, y kopyalar.
// `:` komutları slash komutlarına eşlenir (`:compact` → `/compact`).

export type VimMode = 'insert' | 'normal' | 'visual'

export interface VimState {
  mode: VimMode
  /** "d" bekleyen operatör (dd) */
  pending: '' | 'd'
  /** Görsel kipte seçimin başladığı konum */
  anchor?: number
  /** Son kesilen/kopyalanan metin (p) */
  register?: string
}

export interface VimResult {
  state: VimState
  text: string
  cursor: number
  /** true: tuş vim tarafından işlendi, varsayılan davranış engellenmeli */
  handled: boolean
  /** Görsel kipte gösterilecek seçim [başlangıç, bitiş) */
  selection?: [number, number]
}

export const initialVimState: VimState = { mode: 'insert', pending: '' }

const lineStart = (text: string, pos: number) => text.lastIndexOf('\n', pos - 1) + 1
function lineEnd(text: string, pos: number): number {
  const end = text.indexOf('\n', pos)
  return end === -1 ? text.length : end
}
/** Normal kipte imleç satırın son karakterinde durur (boş satırda satır başında). */
const lastCharOfLine = (text: string, pos: number) => Math.max(lineStart(text, pos), lineEnd(text, pos) - 1)
const clampNormal = (text: string, pos: number) => Math.min(Math.max(pos, lineStart(text, pos)), lastCharOfLine(text, pos))

type CharClass = 'space' | 'word' | 'punct'
function charClass(ch: string | undefined): CharClass {
  if (ch === undefined || /\s/.test(ch)) return 'space'
  return /[\p{L}\p{N}_]/u.test(ch) ? 'word' : 'punct'
}

function nextWordStart(text: string, pos: number): number {
  let i = pos
  const cls = charClass(text[i])
  if (cls !== 'space') while (i < text.length && charClass(text[i]) === cls) i++
  while (i < text.length && charClass(text[i]) === 'space') i++
  return Math.min(i, Math.max(0, text.length - 1))
}

function prevWordStart(text: string, pos: number): number {
  let i = pos - 1
  while (i > 0 && charClass(text[i]) === 'space') i--
  const cls = charClass(text[i])
  while (i > 0 && charClass(text[i - 1]) === cls) i--
  return Math.max(0, i)
}

function verticalMove(text: string, pos: number, dir: 1 | -1): number {
  const start = lineStart(text, pos)
  const column = pos - start
  if (dir === 1) {
    const end = lineEnd(text, pos)
    if (end >= text.length) return pos
    const nextStart = end + 1
    return Math.min(nextStart + column, lastCharOfLine(text, nextStart))
  }
  if (start === 0) return pos
  const prevStart = lineStart(text, start - 1)
  return Math.min(prevStart + column, lastCharOfLine(text, prevStart))
}

const INSERT: VimState = { mode: 'insert', pending: '' }

/**
 * Tek bir tuşu işler. key: KeyboardEvent.key. Değiştirici tuşlu kısayollar (Ctrl+V vb.) çağıran tarafından
 * vim'e iletilmemelidir.
 */
export function vimKey(state: VimState, key: string, text: string, cursor: number): VimResult {
  const result = (s: VimState, t: string, c: number, handled = true): VimResult => ({ state: s, text: t, cursor: c, handled })
  const register = state.register ?? ''
  const normal = (reg = register): VimState => ({ mode: 'normal', pending: '', register: reg })

  if (state.mode === 'visual') {
    const anchor = state.anchor ?? cursor
    const span = (c: number): [number, number] => [Math.min(anchor, c), Math.min(text.length, Math.max(anchor, c) + 1)]
    const extend = (c: number): VimResult => ({ state, text, cursor: c, handled: true, selection: span(c) })
    switch (key) {
      case 'Escape':
      case 'v':
        return result(normal(), text, cursor)
      case 'h':
        return extend(Math.max(0, cursor - 1))
      case 'l':
        return extend(Math.min(Math.max(0, text.length - 1), cursor + 1))
      case 'j':
        return extend(verticalMove(text, cursor, 1))
      case 'k':
        return extend(verticalMove(text, cursor, -1))
      case 'w':
        return extend(nextWordStart(text, cursor))
      case 'b':
        return extend(prevWordStart(text, cursor))
      case '0':
        return extend(lineStart(text, cursor))
      case '$':
        return extend(lastCharOfLine(text, cursor))
      case 'y': {
        const [from, to] = span(cursor)
        return result(normal(text.slice(from, to)), text, from)
      }
      case 'd':
      case 'x': {
        const [from, to] = span(cursor)
        const next = text.slice(0, from) + text.slice(to)
        return result(normal(text.slice(from, to)), next, clampNormal(next, Math.min(from, Math.max(0, next.length - 1))))
      }
      default:
        return { state, text, cursor, handled: key.length === 1, selection: span(cursor) }
    }
  }

  if (state.mode === 'insert') {
    if (key !== 'Escape') return result(state, text, cursor, false)
    // Vim'deki gibi ekleme kipinden çıkınca imleç bir sola kayar.
    return result(normal(), text, cursor > lineStart(text, cursor) ? cursor - 1 : cursor)
  }

  if (state.pending === 'd') {
    if (key !== 'd') return result(normal(), text, cursor)
    const start = lineStart(text, cursor)
    const end = lineEnd(text, cursor)
    // Satırı ayırıcısıyla birlikte sil; son satırsa önceki satır sonunu sil.
    const [from, to] = end < text.length ? [start, end + 1] : [Math.max(0, start - 1), end]
    const next = text.slice(0, from) + text.slice(to)
    return result(normal(text.slice(from, to)), next, clampNormal(next, from === start ? start : lineStart(next, from)))
  }

  switch (key) {
    case 'Escape':
      return result(normal(), text, cursor, false)
    case 'h':
    case 'Backspace':
      return result(normal(), text, Math.max(lineStart(text, cursor), cursor - 1))
    case 'l':
      return result(normal(), text, Math.min(cursor + 1, lastCharOfLine(text, cursor)))
    case 'j':
      return result(normal(), text, verticalMove(text, cursor, 1))
    case 'k':
      return result(normal(), text, verticalMove(text, cursor, -1))
    case 'w':
      return result(normal(), text, nextWordStart(text, cursor))
    case 'b':
      return result(normal(), text, prevWordStart(text, cursor))
    case '0':
      return result(normal(), text, lineStart(text, cursor))
    case '$':
      return result(normal(), text, lastCharOfLine(text, cursor))
    case 'x': {
      if (cursor >= lineEnd(text, cursor)) return result(normal(), text, cursor)
      const next = text.slice(0, cursor) + text.slice(cursor + 1)
      return result(normal(), next, clampNormal(next, cursor))
    }
    case 'd':
      return result({ mode: 'normal', pending: 'd', register }, text, cursor)
    case 'v':
      return { state: { mode: 'visual', pending: '', anchor: cursor, register }, text, cursor, handled: true, selection: [cursor, Math.min(text.length, cursor + 1)] }
    case 'p': {
      if (!register) return result(state, text, cursor)
      const at = Math.min(text.length, cursor + 1)
      return result(state, text.slice(0, at) + register + text.slice(at), at + register.length - 1)
    }
    case ':': {
      // `:` komutları slash komutlarına eşlenir: taslağın başına `/` gelir, ekleme kipine geçilir.
      const next = text.startsWith('/') ? text : `/${text}`
      return result({ mode: 'insert', pending: '', register }, next, 1)
    }
    case 'i':
      return result({ ...INSERT, register }, text, cursor)
    case 'a':
      return result({ ...INSERT, register }, text, Math.min(cursor + 1, lineEnd(text, cursor)))
    case 'A':
      return result({ ...INSERT, register }, text, lineEnd(text, cursor))
    case 'I': {
      const start = lineStart(text, cursor)
      const firstNonBlank = text.slice(start, lineEnd(text, cursor)).search(/\S/)
      return result({ ...INSERT, register }, text, firstNonBlank === -1 ? lineEnd(text, cursor) : start + firstNonBlank)
    }
    case 'o': {
      const end = lineEnd(text, cursor)
      return result({ ...INSERT, register }, `${text.slice(0, end)}\n${text.slice(end)}`, end + 1)
    }
    default:
      // Normal kipte diğer yazdırılabilir tuşlar metne yazılmaz; ok, Enter vb. tarayıcıya kalır.
      return result(normal(), text, cursor, key.length === 1)
  }
}
