// keymap.toml `[composer]` bağlamaları (§28.10, ADR 0050): klavye olayının adı (`ctrl+j`) ve eylemi.
// Ad biçimi Rust tarafındaki `kor_runtime::prefs::normalize_key` ile aynıdır. Saf; node --test ile sınanır.

export const COMPOSER_ACTIONS = [
  'submit', 'submit_now', 'newline', 'cancel', 'mode_cycle', 'keys_cycle', 'line_start', 'line_end',
  'char_back', 'char_forward', 'word_back', 'word_forward', 'line_up', 'line_down', 'kill_line',
  'delete_char', 'delete_word_back', 'yank',
] as const
export type ComposerAction = (typeof COMPOSER_ACTIONS)[number]

export interface KeyLike {
  key: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
}

const NAMED: Record<string, string> = {
  Enter: 'enter', Escape: 'escape', Tab: 'tab', Backspace: 'backspace', Delete: 'delete', ArrowUp: 'up',
  ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', Home: 'home', End: 'end', PageUp: 'pageup',
  PageDown: 'pagedown', Insert: 'insert', ' ': 'space',
}

export function keyName(e: KeyLike): string | null {
  let base = NAMED[e.key]
  let shift = e.shiftKey
  if (!base) {
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(e.key)) base = e.key.toLowerCase()
    else if ([...e.key].length === 1) {
      base = e.key.toLowerCase()
      // Shift'li karakterin kendisi zaten büyük harf/işaret; harflerde shift ayrıca yazılır.
      if (e.key !== base) shift = true
      else if (!/\p{L}/u.test(e.key)) shift = false
    } else return null
  }
  const mods = [e.ctrlKey && 'ctrl', e.altKey && 'alt', shift && 'shift', e.metaKey && 'meta'].filter(Boolean)
  return [...mods, base].join('+')
}

const isAction = (a: string): a is ComposerAction => (COMPOSER_ACTIONS as readonly string[]).includes(a)

/** Kullanıcının bağlaması varsa eylemi. */
export function composerAction(map: Record<string, string>, e: KeyLike): ComposerAction | null {
  const name = keyName(e)
  const a = name ? map[name] : undefined
  return a && isAction(a) ? a : null
}

/** Tuş düzenleri arasında döner (mesaj kutusundaki düğme ve `keys_cycle`). */
export const nextKeys = (k: 'default' | 'vim' | 'emacs'): 'default' | 'vim' | 'emacs' => (k === 'default' ? 'vim' : k === 'vim' ? 'emacs' : 'default')
