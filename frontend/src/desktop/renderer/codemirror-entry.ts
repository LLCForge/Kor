// [g16] Ayrı paket (dist/renderer/codemirror.js): editör ilk kez açılınca yüklenir (bkz. CodeEditor.tsx). CodeMirror'a
// özgü her şey (diller, tema, uzantılar) buradadır; CodeEditor.tsx yalnızca tip içe aktarır.
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { css } from '@codemirror/lang-css'
import { html } from '@codemirror/lang-html'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { markdown } from '@codemirror/lang-markdown'
import { python } from '@codemirror/lang-python'
import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from '@codemirror/language'
import { highlightSelectionMatches, searchKeymap } from '@codemirror/search'
import { EditorState, type Extension, type Text } from '@codemirror/state'
import { drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { tags as t } from '@lezer/highlight'

export interface CreateEditorOptions {
  /** Gölge kök içindeki kap */
  parent: HTMLElement
  root: ShadowRoot
  path: string
  /** Diskteki içerik */
  content: string
  /** Kaydedilmemiş taslak varsa editör onunla açılır */
  draft: Text | undefined
  onSave: () => void
  onDocChanged: (doc: Text) => void
}

export interface CodeMirrorApi {
  /** diskDoc: diskteki içeriğin (satır sonu biçimi korunarak) belgesi; kirli durum buna göre hesaplanır */
  createEditor(options: CreateEditorOptions): { view: EditorView; diskDoc: Text }
  scrollIntoView: typeof EditorView.scrollIntoView
}

function languageFor(path: string): Extension {
  const ext = path.slice(path.lastIndexOf('.') + 1).toLowerCase()
  switch (ext) {
    case 'js':
    case 'mjs':
    case 'cjs':
      return javascript()
    case 'jsx':
      return javascript({ jsx: true })
    case 'ts':
    case 'mts':
    case 'cts':
      return javascript({ typescript: true })
    case 'tsx':
      return javascript({ typescript: true, jsx: true })
    case 'json':
    case 'jsonc':
      return json()
    case 'css':
    case 'scss':
    case 'less':
      return css()
    case 'html':
    case 'htm':
    case 'svg':
    case 'xml':
    case 'vue':
      return html()
    case 'md':
    case 'markdown':
    case 'mdx':
      return markdown()
    case 'py':
    case 'pyw':
      return python()
    default:
      return []
  }
}

/** Dosya tamamen CRLF ise korunur; karışık/LF dosyalar \n ile kaydedilir. */
function lineSeparatorOf(text: string): string | null {
  const crlf = text.split('\r\n').length - 1
  const lf = text.split('\n').length - 1
  return crlf > 0 && crlf === lf ? '\r\n' : null
}

// CSP (style-src 'self') <style> eklemeye izin vermez; CodeMirror belge kökünde <style> kullanır, gölge kökte ise
// oluşturulabilir stil sayfası (adoptedStyleSheets) kullanır. Bu yüzden editör gölge DOM'a kurulur ve tüm görünüm
// tema/vurgu tanımlarıyla verilir; renkler sayfanın CSS değişkenlerinden (gölge sınırını geçer) gelir.
const editorTheme = EditorView.theme({
  '&': { height: '100%', fontSize: '12px', backgroundColor: 'var(--surface)', color: 'var(--text)' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '.cm-gutters': { backgroundColor: 'var(--bg)', color: 'var(--subtle)', borderRight: '1px solid var(--border)' },
  '.cm-activeLine': { backgroundColor: 'var(--hover)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--hover)', color: 'var(--text)' },
  '&.cm-focused': { outline: 'none' },
  '.cm-cursor': { borderLeftColor: 'var(--accent)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: 'var(--accent-soft) !important' },
  '.cm-panels': { backgroundColor: 'var(--bg)', color: 'var(--text)', fontFamily: 'var(--font-sans)' },
  '.cm-searchMatch': { backgroundColor: 'var(--accent-soft)', outline: '1px solid var(--accent)' },
  '.cm-matchingBracket': { backgroundColor: 'var(--accent-soft)', outline: '1px solid var(--subtle)' },
})

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword], color: 'var(--accent)' },
  { tag: [t.string, t.special(t.string), t.regexp, t.inserted], color: 'var(--add-text)' },
  { tag: [t.number, t.bool, t.atom, t.null, t.literal], color: 'var(--warn)' },
  { tag: [t.comment, t.meta], color: 'var(--subtle)', fontStyle: 'italic' },
  { tag: [t.typeName, t.className, t.namespace], color: 'var(--g4-syn-type)' },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.definition(t.function(t.variableName))], color: 'var(--g4-syn-fn)' },
  { tag: [t.tagName, t.heading], color: 'var(--accent)', fontWeight: '600' },
  { tag: [t.attributeName, t.propertyName], color: 'var(--text-soft)' },
  { tag: [t.invalid, t.deleted], color: 'var(--del-text)' },
  { tag: [t.link, t.url], color: 'var(--accent)', textDecoration: 'underline' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: t.strong, fontWeight: '700' },
])

function createEditor({ parent, root, path, content, draft, onSave, onDocChanged }: CreateEditorOptions): { view: EditorView; diskDoc: Text } {
  const separator = lineSeparatorOf(content)
  const base: Extension[] = separator ? [EditorState.lineSeparator.of(separator)] : []
  const diskDoc = EditorState.create({ doc: content, extensions: base }).doc
  const state = EditorState.create({
    doc: draft ?? diskDoc,
    extensions: [
      base,
      lineNumbers(),
      highlightActiveLineGutter(),
      history(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      syntaxHighlighting(highlight),
      languageFor(path),
      editorTheme,
      EditorView.contentAttributes.of({ 'aria-label': `${path} düzenleyicisi` }),
      keymap.of([
        {
          key: 'Mod-s',
          preventDefault: true,
          run: () => {
            onSave()
            return true
          },
        },
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) onDocChanged(update.state.doc)
      }),
    ],
  })
  return { view: new EditorView({ state, parent, root }), diskDoc }
}

const codeMirror: CodeMirrorApi = { createEditor, scrollIntoView: EditorView.scrollIntoView }
;(window as { __g16CodeMirror?: CodeMirrorApi }).__g16CodeMirror = codeMirror
