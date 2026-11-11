import { ArrowUp, Eye, FileText, Folder, ListPlus, PenLine, SquareSlash, X } from 'lucide-react'
import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react'
import useSWR from 'swr'
import type { PermissionMode } from '../../core/permissions.ts'
import type { ModelInfo } from '../ipc.ts'
import { api, errorMessage } from './api.ts'
import { onNativeDrop } from './kor-bridge.ts'
import { AttachmentCards } from './g2/AttachmentCards.tsx'
import { detectTrigger, estimateTokens, moveItem, replaceRange } from './g2/composer-utils.ts'
import { EffortPicker } from './g2/EffortPicker.tsx'
import { FastToggle } from './g2/FastToggle.tsx'
import { IMAGE_TYPES, MAX_IMAGES, readImageFiles } from './g2/images.ts'
import { expandPaste, expandPastes, pasteLabel, pastesInDraft, shouldFold } from './g2/paste.ts'
import { setG2Prefs, useG2Prefs } from './g2/prefs.ts'
import { emacsKey, initialEmacsState } from './prefs/emacs.ts' // [ADR 0050] emacs tuş düzeni
import { composerAction, nextKeys } from './prefs/keymap.ts' // [ADR 0050] keymap.toml
import { prefsApi } from './prefs/api.ts'
import { QueueList } from './g2/QueueList.tsx'
import { SuggestionMenu, type SuggestionItem } from './g2/SuggestionMenu.tsx'
import { g2Bridge } from './g2/use-g2-app.ts'
import { initialVimState, vimKey, type VimState } from './g2/vim.ts'
import { Markdown } from './Markdown.tsx'
import { ModelPicker } from './ModelPicker.tsx'
import { ModeMenu, MODES, nextMode } from './ModeMenu.tsx'
import { FolderMenu } from './FolderMenu.tsx'
import { ProfileMenu } from './ProfileMenu.tsx' // [cdx]
import { ComposerMore } from './ComposerMore.tsx'
import { AttachMenu } from './AttachMenu.tsx'
import { Segmented, Tooltip } from './kor-ui/index.ts'
import { filterSlashCommands, findSlashCommand, parseSlashInput, SLASH_COMMANDS, type SlashCommand, type SlashCommandContext } from './slash-commands.ts'
import { expandCommand } from '../../core/command-expand.ts'
import type { FileCommand } from '../../core/file-commands.ts'
import { UsageMeter } from './UsageMeter.tsx'
import { ContextMeter } from './ContextMeter.tsx' // [g6]
import { EstimateChip } from './estimate/EstimateChip.tsx' // §27.16
import { OutputStyleMenu } from './OutputStyleMenu.tsx' // [g3]
import { G22ModelSupport } from './G22ModelSupport.tsx' // [g22]
import { CostChip } from './g9/CostChip.tsx' // [g9]
import { VoiceButton } from './panels/visual/VoiceButton.tsx' // [j4]
import { findPanel } from './panels/registry.ts'
import type { ParityCommand } from './parity-commands.gen.ts'
import { panelIdsFor } from './parity-palette.ts'
import { paritySlashCommands } from './parity-slash.ts'
import { ParityRunner } from './ParityRunner.tsx'
import { clearReviewComments, useReviewComments } from './review-comments.ts'
import { TAGS_KEY } from './TagsSection.tsx'
import { Tag as TagIcon } from 'lucide-react'
import { MessageSquare, SquareDashedMousePointer, Trash2 } from 'lucide-react'
import { clearPreviewSelections, removePreviewSelection, selectionLabel, usePreviewSelections } from './preview-store.ts' // [cdx]
import { useIntentGate } from './intent/IntentProposal.tsx' // [k7] §30 niyet katmanı
import { confirmAction } from './ConfirmDialog.tsx' // [osint]
import { osextraApi } from './osint/osextra-bridge.ts' // [osint] §27.10 Universal Clipboard
import { uiLocale } from './ui-locale.ts'

interface ComposerProps {
  draft: string
  onDraftChange: (text: string) => void
  running: boolean
  disabled: boolean
  attachments: string[]
  onPickFiles: () => void
  onRemoveAttachment: (path: string) => void
  onSend: (text: string) => void
  onAbort: () => void
  model: string
  models: ModelInfo[] | undefined
  modelsError: string | null
  onModelChange: (id: string) => void
  permissionMode: PermissionMode
  defaultPermissionMode: PermissionMode
  onPermissionModeChange: (mode: PermissionMode) => void
  onMakeDefaultMode: (mode: PermissionMode) => void
  usageRefreshKey: number
  /** Kullanım penceresinde bu oturumun token/maliyeti gösterilir */
  sessionId: string | null
  /** Çalışma klasörü seçici (Klasör yok / son kullanılanlar / Klasör aç) */
  root: string | null
  recentWorkspaces: readonly string[]
  onOpenFolder: () => void
  onClearFolder: () => void
  onPickRecentFolder: (path: string) => void
  /** cdesktop: mesaj kutusunun üstündeki durum şeridi (dosya +/-, görevler, oturum seçici) */
  statusRow?: ReactNode
}

const MAX_ATTACHMENTS = 20
// §10 (ADR 0048): görseller gönderilirken yerel OCR ile metne çevrilir; görselsiz modeller de okur.
const OCR_NOTE = 'Görseller gönderilirken yerel OCR ile metne çevrilip mesaja eklenir (LLM\'siz); model görüntünün kendisini almaz.'

// Katlanan yapıştırmalar modül düzeyinde tutulur: mesaj kutusu yeniden bağlansa da (boş ekran -> sohbet) taslaktaki
// yer tutucular açılabilsin. Kimlikler uygulama boyunca benzersizdir; gönderilenler silinir.
const pastes = new Map<number, string>()
let pasteSeq = 0

interface MenuEntry extends SuggestionItem {
  apply: () => void
}

export function Composer(props: ComposerProps) {
  const { draft, onDraftChange, running, disabled, attachments } = props
  // Yan yana görünümde iki mesaj kutusu olabilir: kimlik bileşene özgü olmalı.
  const promptId = useId()
  const bridge = g2Bridge.current
  const prefs = useG2Prefs()
  // §28.10: tuş düzeni yapılandırmadan (`keys.mode` / `ui.tui_keys`, TUI ile ortak); okunamazsa yerel tercih.
  const { data: uiPrefs, mutate: mutateUi } = useSWR('prefs:ui', () => prefsApi.ui(), { revalidateOnFocus: false })
  const { data: keymap } = useSWR('prefs:keymap', () => prefsApi.keymap(), { revalidateOnFocus: false })
  const keyMode = uiPrefs?.keys ?? (prefs.vim ? 'vim' : 'default')
  const vimOn = keyMode === 'vim'
  const [emacs, setEmacs] = useState(initialEmacsState)
  const [parityRun, setParityRun] = useState<{ command: ParityCommand; rest: string } | null>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const pendingCaret = useRef<number | null>(null)
  const pendingSelection = useRef<[number, number] | null>(null)
  const menuId = useId()
  const [composing, setComposing] = useState(false)
  const [caret, setCaret] = useState(0)
  const [menuIndex, setMenuIndex] = useState(0)
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [vim, setVim] = useState<VimState>(initialVimState)
  const [tab, setTab] = useState<'write' | 'preview'>('write')
  const [fullscreen, setFullscreen] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const [dragOver, setDragOver] = useState(false)
  const reviewComments = useReviewComments()
  const intentGate = useIntentGate() // [k7]
  const previewSelections = usePreviewSelections() // [cdx] tarayıcı companion seçimleri

  const images = bridge?.images ?? []
  const queued = bridge?.queued ?? []
  const { data: caps } = useSWR('g2:caps', () => api.g2ModelCaps(), { revalidateOnFocus: false })
  const modelCaps = caps?.[props.model]
  const { data: stats } = useSWR(attachments.length > 0 ? ['g2:stat', ...attachments] : null, () => api.g2StatFiles(attachments))

  const expanded = useMemo(() => expandPastes(draft, pastes), [draft])
  const pasteIds = pastesInDraft(draft, pastes)
  const fileChars = (stats ?? []).reduce((sum, s) => sum + (s.size ?? 0), 0)
  const tokens = estimateTokens(expanded.length + fileChars)

  const hasContent = expanded.trim().length > 0 || images.length > 0 || attachments.length > 0 || reviewComments.length > 0 || previewSelections.length > 0
  const canSend = !disabled && hasContent && (bridge !== null || !running)

  // Programatik metin değişikliklerinden sonra imleç konumu geri yüklenir.
  useLayoutEffect(() => {
    const sel = pendingSelection.current
    if (sel && inputRef.current) {
      pendingSelection.current = null
      pendingCaret.current = null
      inputRef.current.setSelectionRange(sel[0], sel[1])
      return
    }
    const caretTo = pendingCaret.current
    if (caretTo === null || !inputRef.current) return
    pendingCaret.current = null
    inputRef.current.setSelectionRange(caretTo, caretTo)
  })

  useEffect(() => {
    if (tab === 'write') inputRef.current?.focus()
  }, [tab])

  const setText = (text: string, cursor: number) => {
    pendingCaret.current = cursor
    setCaret(cursor)
    onDraftChange(text)
  }

  const abort = async () => {
    try {
      if (bridge) await bridge.abort()
      else props.onAbort()
    } catch (err) {
      setNote(errorMessage(err))
    }
  }

  const slashCtx: SlashCommandContext = {
    sessionId: bridge?.activeId ?? props.sessionId ?? '',
    running,
    models: props.models ?? [],
    permissionModes: MODES,
    newChat: () => bridge?.newChat(),
    setModel: props.onModelChange,
    setPermissionMode: props.onPermissionModeChange,
    openMemory: () => bridge?.openMemory(),
    openSystemPrompt: () => bridge?.openSystemPrompt(),
    pickFiles: props.onPickFiles,
    abort: () => void abort(),
    setDraft: onDraftChange,
    notify: setNote,
    // [g6] /compact ve /init
    compact: async () => {
      if (!slashCtx.sessionId) return setNote('Önce bir mesaj gönder; sıkıştırılacak konuşma yok.')
      setNote('Bağlam sıkıştırılıyor…')
      const result = await api.compactSession(slashCtx.sessionId)
      setNote(result.compacted ? `Bağlam sıkıştırıldı: ${result.beforeTokens.toLocaleString(uiLocale())} → ${result.afterTokens.toLocaleString(uiLocale())} token` : `Sıkıştırılmadı: ${result.reason ?? 'gerek yok'}`)
    },
    initProject: async () => {
      setNote('AGENTS.md hazırlanıyor (proje haritası yoksa önce o oluşturulur)…')
      const result = await api.initProject(false)
      setNote(result.status === 'created' ? `${result.path} oluşturuldu.` : `${result.path} zaten var; üzerine yazmak için Proje haritası penceresini kullan.`)
    },
  }

  // Dosyadan slash komutları (Claude/Cursor uyumlu): yerleşiklerle birleşir; çalışınca gövdeyi genişletip taslağa yazar.
  const { data: fileCommands } = useSWR('commands', () => api.commands.list(), { revalidateOnFocus: true })
  const allCommands = useMemo<readonly SlashCommand[]>(() => {
    const builtinNames = new Set(SLASH_COMMANDS.map((c) => c.name))
    const fromFiles = (fileCommands ?? [])
      .filter((c: FileCommand) => !builtinNames.has(c.name))
      .map(
        (c: FileCommand): SlashCommand => ({
          name: c.name,
          description: c.description,
          source: c.source,
          ...(c.argumentHint ? { argHint: c.argumentHint } : {}),
          run: (ctx, args) => {
            ctx.setDraft(expandCommand(c.body, args))
            ctx.notify('Komut taslağa yazıldı; göndermek için Enter.')
          },
        }),
      )
    const taken = new Set([...SLASH_COMMANDS, ...fromFiles].map((c) => c.name))
    // Kor'un bütün komutları (`/seo audit`, `/cost why`…): paneli olan paneli açar, olmayan çalıştırıcıyı gösterir.
    const fromKor = paritySlashCommands(taken, (command, rest) => {
      const panel = panelIdsFor(command.method.split('.')[0]!).map(findPanel).find((p) => p !== undefined)
      if (panel) window.dispatchEvent(new CustomEvent('kor:panel-open', { detail: panel.id }))
      else setParityRun({ command, rest })
    })
    return [...SLASH_COMMANDS, ...fromFiles, ...fromKor]
  }, [fileCommands])

  const runCommand = (command: SlashCommand, args: string) => {
    onDraftChange('')
    setNote(null)
    Promise.resolve()
      .then(() => command.run(slashCtx, args))
      .catch((err: unknown) => setNote(errorMessage(err)))
  }

  const addAttachments = (paths: readonly string[]) => {
    if (!bridge) return
    bridge.setAttachments((prev) => {
      const next = [...new Set([...prev, ...paths])]
      if (next.length > MAX_ATTACHMENTS) setNote(`En fazla ${MAX_ATTACHMENTS} ek eklenebilir.`)
      return next.slice(0, MAX_ATTACHMENTS)
    })
  }

  const addImages = async (files: readonly File[]) => {
    if (!bridge) return
    const { images: read, errors } = await readImageFiles(files, MAX_IMAGES - images.length)
    if (read.length > 0) bridge.setImages((prev) => [...prev, ...read])
    setNote(errors.length > 0 ? errors.join(' ') : null)
  }

  // --- Açılır menü: "/" komutları, komut argümanları, "@" dosya/klasör ---
  const trigger = tab === 'write' && !disabled ? detectTrigger(draft, caret) : null
  const triggerKey = trigger ? `${trigger.kind}:${trigger.start}` : null
  const argCommand = trigger?.kind === 'commandArg' ? findSlashCommand(allCommands, trigger.command) : undefined
  const menuOpen = trigger !== null && triggerKey !== dismissed && (trigger.kind !== 'commandArg' || argCommand?.suggest !== undefined)
  const mentionQuery = menuOpen && trigger?.kind === 'mention' ? trigger.query : null
  const { data: hits, isLoading: hitsLoading } = useSWR(
    mentionQuery !== null ? (['g2:files', mentionQuery] as const) : null,
    ([, query]) => api.g2SearchFiles(query),
    { keepPreviousData: true },
  )
  // [cdx] Etiketler: @ad eşleşmeleri dosya listesinin üstünde (cdesktop TAGS / FILES)
  const { data: tags } = useSWR(mentionQuery !== null ? TAGS_KEY : null, () => api.tags.list(), { revalidateOnFocus: false })

  useEffect(() => setMenuIndex(0), [triggerKey, trigger?.query])

  const entries: MenuEntry[] = []
  if (menuOpen && trigger?.kind === 'command') {
    for (const command of filterSlashCommands(allCommands, trigger.query)) {
      const tag = command.source === 'plugin' ? 'eklenti' : command.source ? 'dosya' : null
      entries.push({
        key: command.name,
        label: `/${command.name}${command.argHint ? ` ${command.argHint}` : ''}`,
        description: tag ? `${command.description} · ${tag}` : command.description,
        icon: <SquareSlash size={14} />,
        apply: () => (command.suggest ? setText(`/${command.name} `, command.name.length + 2) : runCommand(command, '')),
      })
    }
  } else if (menuOpen && trigger?.kind === 'commandArg' && argCommand?.suggest) {
    for (const s of argCommand.suggest(slashCtx, trigger.query)) {
      entries.push({ key: s.value, label: s.label, ...(s.description ? { description: s.description } : {}), apply: () => runCommand(argCommand, s.value) })
    }
  } else if (menuOpen && trigger?.kind === 'mention') {
    const q = trigger.query.toLocaleLowerCase('tr')
    for (const tag of (tags ?? []).filter((t) => t.name.toLocaleLowerCase('tr').includes(q)).slice(0, 5)) {
      entries.push({
        key: `tag:${tag.name}`,
        label: `@${tag.name}`,
        description: `Etiket · ${tag.content.replace(/\s+/g, ' ').slice(0, 80)}`,
        icon: <TagIcon size={14} />,
        apply: () => {
          const { text, cursor } = replaceRange(draft, trigger.start, trigger.end, `${tag.content} `)
          setText(text, cursor)
        },
      })
    }
    for (const hit of hits ?? []) {
      entries.push({
        key: hit.path,
        label: hit.isDir ? `${hit.path}/` : hit.path,
        description: hit.isDir ? 'Klasör ağacı eklenir (derinlik 3)' : 'Dosya eklenir',
        icon: hit.isDir ? <Folder size={14} /> : <FileText size={14} />,
        apply: () => {
          const { text, cursor } = replaceRange(draft, trigger.start, trigger.end, `@${hit.path}${hit.isDir ? '/' : ''} `)
          setText(text, cursor)
          addAttachments([hit.path])
        },
      })
    }
  }
  const activeIndex = Math.min(menuIndex, Math.max(0, entries.length - 1))
  const menuTitle = trigger?.kind === 'mention' ? 'Dosya ve klasörler' : trigger?.kind === 'commandArg' ? `/${trigger.command}` : 'Komutlar'

  // --- Gönderim ---
  const submit = (interrupt: boolean) => {
    // Görsel desteklenmiyorsa uyarı zaten kalıcı olarak gösteriliyor.
    if (!canSend) return
    const text = expanded.trim()
    const slash = images.length === 0 && attachments.length === 0 ? parseSlashInput(text) : null
    const command = slash ? findSlashCommand(allCommands, slash.name) : undefined
    if (command && slash) {
      runCommand(command, slash.args)
      return
    }
    setNote(null)
    for (const id of pasteIds) pastes.delete(id)
    const deliver = (t: string) => {
      if (!bridge) {
        props.onSend(t)
        return
      }
      bridge.send({ text: t, interrupt: interrupt && running }).catch((err: unknown) => setNote(errorMessage(err)))
    }
    // [k7] §30: serbest metin önce niyet katmanından geçer; Kor komutu değilse aynen gönderilir.
    if (!running && images.length === 0 && attachments.length <= 1 && intentGate.gate(text, deliver, attachments[0])) {
      setText('', 0)
      return
    }
    deliver(text)
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (composing || e.nativeEvent.isComposing) return
    if (menuOpen && entries.length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        const step = e.key === 'ArrowDown' ? 1 : -1
        setMenuIndex((activeIndex + step + entries.length) % entries.length)
        return
      }
      if ((e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) || (e.key === 'Tab' && !e.shiftKey)) {
        e.preventDefault()
        entries[activeIndex]?.apply()
        return
      }
    }
    if (menuOpen && e.key === 'Escape') {
      e.preventDefault()
      setDismissed(triggerKey)
      return
    }
    // §28.10 keymap.toml `[composer]`: kullanıcının bağlamaları yerleşik tuşlardan önce gelir.
    const bound = keymap ? composerAction(keymap.composer, e) : null
    if (bound) {
      const el = e.currentTarget
      e.preventDefault()
      switch (bound) {
        case 'submit':
          submit(false)
          return
        case 'submit_now':
          submit(true)
          return
        case 'newline':
          setText(`${draft.slice(0, el.selectionStart)}\n${draft.slice(el.selectionEnd)}`, el.selectionStart + 1)
          return
        case 'cancel':
          if (running) void abort()
          return
        case 'mode_cycle':
          props.onPermissionModeChange(nextMode(props.permissionMode))
          return
        case 'keys_cycle':
          toggleVim()
          return
        default: {
          const emacsName: Record<string, { key: string; ctrl: boolean; alt: boolean }> = {
            line_start: { key: 'a', ctrl: true, alt: false }, line_end: { key: 'e', ctrl: true, alt: false },
            char_back: { key: 'b', ctrl: true, alt: false }, char_forward: { key: 'f', ctrl: true, alt: false },
            word_back: { key: 'b', ctrl: false, alt: true }, word_forward: { key: 'f', ctrl: false, alt: true },
            line_up: { key: 'p', ctrl: true, alt: false }, line_down: { key: 'n', ctrl: true, alt: false },
            kill_line: { key: 'k', ctrl: true, alt: false }, delete_char: { key: 'd', ctrl: true, alt: false },
            delete_word_back: { key: 'w', ctrl: true, alt: false }, yank: { key: 'y', ctrl: true, alt: false },
          }
          const k = emacsName[bound]
          if (k) {
            const r = emacsKey(emacs, k, draft, el.selectionStart)
            setEmacs(r.state)
            setText(r.text, r.cursor)
          }
          return
        }
      }
    }
    // §28.10 emacs düzeni: Ctrl/Alt kısayolları metni düzenler (Ctrl+Enter vb. aşağıda kalır).
    if (keyMode === 'emacs' && (e.ctrlKey || e.altKey) && !e.metaKey && e.key !== 'Enter') {
      const r = emacsKey(emacs, { key: e.key, ctrl: e.ctrlKey, alt: e.altKey }, draft, e.currentTarget.selectionStart)
      if (r.handled) {
        e.preventDefault()
        setEmacs(r.state)
        if (r.text !== draft) setText(r.text, r.cursor)
        else {
          pendingCaret.current = r.cursor
          setCaret(r.cursor)
        }
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      // [cdx] Ayarlar > Mesaj girişi: 'ctrlEnter' seçiliyse düz Enter yeni satırdır, yalnızca Ctrl/⌘+Enter gönderir.
      if (prefs.sendKey === 'ctrlEnter' && !e.ctrlKey && !e.metaKey) return
      e.preventDefault()
      // Çalışırken Enter sıraya alır, Ctrl+Enter turu kesip hemen gönderir.
      submit(e.ctrlKey || e.metaKey)
      return
    }
    // Claude Code'daki gibi Shift+Tab izin modları arasında döner.
    if (e.key === 'Tab' && e.shiftKey) {
      e.preventDefault()
      props.onPermissionModeChange(nextMode(props.permissionMode))
      return
    }
    if (vimOn && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const result = vimKey(vim, e.key, draft, e.currentTarget.selectionStart)
      if (JSON.stringify(result.state) !== JSON.stringify(vim)) setVim(result.state)
      if (result.handled) {
        e.preventDefault()
        if (result.text !== draft) setText(result.text, result.cursor)
        else {
          pendingCaret.current = result.cursor
          setCaret(result.cursor)
        }
        if (result.selection) pendingSelection.current = result.selection
        return
      }
    }
    if (e.key === 'Escape') {
      if (fullscreen) {
        e.preventDefault()
        setFullscreen(false)
      } else if (running) {
        e.preventDefault()
        void abort()
      }
    }
  }

  const offerRemoteContext = async (text: string) => {
    const remote = await osextraApi.remoteClipboard().catch(() => false)
    if (!remote) return
    const ok = await confirmAction({
      title: 'iPhone panosu',
      message: "iPhone'dan yapıştırdın, bağlam olarak ekleyeyim mi?",
      confirmLabel: 'Bağlam olarak ekle',
    })
    const el = inputRef.current
    if (!ok || !el) return
    const at = el.value.lastIndexOf(text)
    if (at < 0) return
    const id = ++pasteSeq
    pastes.set(id, text)
    const { text: next, cursor } = replaceRange(el.value, at, at + text.length, pasteLabel(text, id))
    setText(next, cursor)
  }

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...e.clipboardData.files].filter((f) => f.type.startsWith('image/'))
    if (files.length > 0 && bridge) {
      e.preventDefault()
      void addImages(files)
      return
    }
    const text = e.clipboardData.getData('text/plain')
    if (!shouldFold(text)) {
      // [osint] §27.10 macOS Universal Clipboard: iPhone'dan gelen metin istenirse bağlam eki olur.
      if (text) void offerRemoteContext(text)
      return
    }
    e.preventDefault()
    const id = ++pasteSeq
    pastes.set(id, text)
    const { selectionStart, selectionEnd } = e.currentTarget
    const { text: next, cursor } = replaceRange(draft, selectionStart, selectionEnd, pasteLabel(text, id))
    setText(next, cursor)
  }

  const isFileDrag = (e: DragEvent) => e.dataTransfer.types.includes('Files')

  const onDrop = (e: DragEvent<HTMLFormElement>) => {
    if (!isFileDrag(e)) return
    e.preventDefault()
    setDragOver(false)
    if (disabled || !bridge) return
    const files = [...e.dataTransfer.files]
    const imageFiles = files.filter((f) => IMAGE_TYPES.includes(f.type))
    const others = files.filter((f) => !IMAGE_TYPES.includes(f.type))
    if (imageFiles.length > 0) void addImages(imageFiles)
    if (others.length === 0) return
    // Yol yalnızca preload'daki webUtils ile alınır; ana süreç çalışma dizini ve hassas dosya kontrolü yapar.
    const paths = others.map((f) => api.g2PathForFile(f)).filter((p) => p.length > 0)
    if (paths.length === 0) {
      setNote('Sürüklenen öğelerin dosya yolu alınamadı.')
      return
    }
    attachPaths(paths)
  }

  const attachPaths = (paths: string[]) => {
    api
      .g2ResolveDropped(paths)
      .then((result) => {
        addAttachments(result.accepted)
        if (result.rejected.length > 0) setNote(`Eklenmedi: ${result.rejected.map((r) => `${r.name} (${r.reason})`).join(', ')}`)
      })
      .catch((err: unknown) => setNote(errorMessage(err)))
  }

  // İşletim sisteminden sürüklenen dosyalar HTML drop'a gelmez (Tauri yakalar); yolları buradan alınır.
  // Yalnızca bu yazma alanının üstüne bırakılanlar eklenir (bölünmüş görünümde iki alan olabilir).
  const formRef = useRef<HTMLFormElement>(null)
  const dropRef = useRef({ disabled, bridge, attachPaths })
  dropRef.current = { disabled, bridge, attachPaths }
  useEffect(
    () =>
      onNativeDrop((d) => {
        const r = formRef.current?.getBoundingClientRect()
        const inside = r !== undefined && d.x >= r.left && d.x <= r.right && d.y >= r.top && d.y <= r.bottom
        if (d.type === 'over') return setDragOver(inside)
        setDragOver(false)
        const { disabled: off, bridge: ok, attachPaths: attach } = dropRef.current
        if (d.type === 'drop' && inside && !off && ok && d.paths.length > 0) attach(d.paths)
        // §21.1: bırakma yazma alanının ve başka bir bırakma bölgesinin dışındaysa uygulama karar verir
        // (klasör → proje olarak açılır, dosyalar → eklere). Yalnızca ana yazma alanı yayınlar.
        else if (d.type === 'drop' && !inside && ok && d.paths.length > 0 && document.querySelector('form.composer') === formRef.current && !document.elementFromPoint(d.x, d.y)?.closest('[data-kor-dropzone], [data-kor-drop]'))
          window.dispatchEvent(new CustomEvent('kor:drop-outside', { detail: d.paths }))
      }),
    [],
  )

  // Düğme varsayılan → vim → emacs arasında döner; seçim yapılandırmaya yazılır (TUI de aynısını okur).
  const toggleVim = () => {
    const next = nextKeys(keyMode)
    setG2Prefs({ vim: next === 'vim' })
    setVim(initialVimState)
    setEmacs(initialEmacsState)
    void prefsApi.setUi('keys', next).then((u) => mutateUi(u, { revalidate: false })).catch((err: unknown) => setNote(errorMessage(err)))
    inputRef.current?.focus()
  }

  // §23.3: yer tutucu başlığı tekrar etmez; kısayolları söyler (gönderme tuşu tercihine göre).
  const sendHint = prefs.sendKey === 'ctrlEnter' ? 'Ctrl+↵ gönder, ↵ yeni satır' : '↵ gönder, ⇧↵ yeni satır'
  const placeholder = disabled
    ? 'Başlamak için bir model/sağlayıcı anahtarı ekle (Ayarlar)'
    : running && bridge
      ? 'Ajan çalışıyor: mesajın sıraya alınır (Ctrl+Enter: şimdi gönder)'
      : `Bir görev yaz — / komutlar, @ dosyalar, ${sendHint}`

  return (
    <>
    <form
      ref={formRef}
      data-kor-focus-host=""
      className={`composer g2-composer${fullscreen ? ' g2-fullscreen' : ''}${dragOver ? ' g2-drag-over' : ''}${running ? ' kor-running' : ''}`}
      data-kor-drop="" // §21.1: dosya bırakmanın izinli olduğu bölge
      onSubmit={(e: FormEvent) => {
        e.preventDefault()
        submit(false)
      }}
      onDragOver={(e) => {
        if (!isFileDrag(e)) return
        e.preventDefault()
        e.dataTransfer.dropEffect = disabled ? 'none' : 'copy'
        setDragOver(true)
      }}
      onDragLeave={(e) => {
        if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) setDragOver(false)
      }}
      onDrop={onDrop}
    >
      {reviewComments.length > 0 && (
        <p className="cdx-review-banner" role="status">
          <MessageSquare size={14} aria-hidden="true" />
          <span>{reviewComments.length} inceleme yorumu eklenecek</span>
          <Tooltip text="Yorumları sil">
            <button type="button" className="action-btn" onClick={clearReviewComments} aria-label="İnceleme yorumlarını sil">
              <Trash2 size={13} aria-hidden="true" />
            </button>
          </Tooltip>
        </p>
      )}
      {previewSelections.length > 0 && (
        <div className="cdx-preview-chips" role="status" aria-label="Önizlemede seçilen öğeler">
          <SquareDashedMousePointer size={13} aria-hidden="true" />
          {previewSelections.map((s) => (
            <span key={s.id} className="cdx-preview-chip" title={`${s.selector}${s.source ? `\n${s.source.file}:${s.source.line}` : ''}`}>
              {selectionLabel(s)}
              <button type="button" className="cdx-preview-chip-x" onClick={() => removePreviewSelection(s.id)} aria-label="Seçimi kaldır"><X size={11} aria-hidden="true" /></button>
            </span>
          ))}
          <Tooltip text="Tümünü kaldır">
            <button type="button" className="action-btn" onClick={clearPreviewSelections} aria-label="Tüm seçimleri kaldır"><Trash2 size={13} aria-hidden="true" /></button>
          </Tooltip>
        </div>
      )}
      {intentGate.element}
      {props.statusRow}
      {queued.length > 0 && bridge && (
        <QueueList
          items={queued}
          onSendNow={(id) => void bridge.sendQueuedNow(id).catch((err: unknown) => setNote(errorMessage(err)))}
          onUndo={(id) => void bridge.unqueue(id).catch((err: unknown) => setNote(errorMessage(err)))}
        />
      )}

      {/* §23.3 üst satır: Yaz/Önizle segmenti + göstergeler (bağlam, kullanım, model desteği, sağlık) + Gelişmiş */}
      <div className="g2-toolbar kor-composer-top">
        <Segmented
          label="Taslak görünümü"
          value={tab}
          onChange={setTab}
          options={[
            { value: 'write', label: 'Yaz', icon: <PenLine size={12} aria-hidden="true" /> },
            { value: 'preview', label: 'Önizle', icon: <Eye size={12} aria-hidden="true" /> },
          ]}
        />
        {vimOn && <span className={`g2-vim-mode${vim.mode !== 'insert' ? ' normal' : ''}`}>{vim.mode === 'normal' ? 'NORMAL' : vim.mode === 'visual' ? 'GÖRSEL' : 'EKLE'}</span>}
        {keyMode === 'emacs' && <span className="g2-vim-mode">EMACS</span>}
        <span className="kor-spacer" />
        {/* [g6] */}
        <ContextMeter sessionId={props.sessionId} running={running} />
        {/* §27.16 süre/maliyet tahmini */}
        <EstimateChip draft={draft} sessionId={props.sessionId} model={props.model} running={running} />
        {/* [g7] */}
        <UsageMeter refreshKey={props.usageRefreshKey} sessionId={props.sessionId} />
        {/* [g22] */}
        <G22ModelSupport model={props.model} attachments={attachments} />
        {/* [g14] Sağlık rozeti ve son yanıt değerlendirmesi G14Layer'dan bu alana portal ile çizilir. */}
        <span className="g14-slot" ref={(el) => { if (el) window.dispatchEvent(new CustomEvent('g14:slot', { detail: el })) }} />
        <ComposerMore
          effort={<EffortPicker options={modelCaps?.reasoningEfforts ?? []} value={prefs.effort} onChange={(effort) => setG2Prefs({ effort })} />}
          fast={<FastToggle model={props.model} models={props.models} onChange={props.onModelChange} />}
          outputStyle={<OutputStyleMenu />}
          profile={
            <ProfileMenu
              permissionMode={props.permissionMode}
              defaultMode={props.defaultPermissionMode}
              model={props.model}
              onPermissionModeChange={props.onPermissionModeChange}
              onModelChange={props.onModelChange}
            />
          }
          vim={keyMode !== 'default'}
          onToggleVim={toggleVim}
          fullscreen={fullscreen}
          onToggleFullscreen={() => setFullscreen((f) => !f)}
        />
      </div>

      <div className="g2-input-wrap">
        {menuOpen && (entries.length > 0 || trigger?.kind === 'mention') && (
          <SuggestionMenu
            id={menuId}
            title={menuTitle}
            items={entries}
            activeIndex={activeIndex}
            loading={hitsLoading}
            emptyText={trigger?.kind === 'mention' ? 'Eşleşen dosya yok' : 'Eşleşen komut yok'}
            onSelect={(i) => entries[i]?.apply()}
            onHover={setMenuIndex}
          />
        )}
        {tab === 'write' ? (
          <>
            <label htmlFor={promptId} className="sr-only">Mesaj</label>
            <textarea
              id={promptId}
              ref={inputRef}
              className={`composer-input${vimOn && vim.mode !== 'insert' ? ' g2-vim-normal' : ''}`}
              rows={1}
              value={draft}
              onChange={(e) => {
                onDraftChange(e.target.value)
                setCaret(e.target.selectionStart)
              }}
              onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
              onKeyDown={onKeyDown}
              onPaste={onPaste}
              onCompositionStart={() => setComposing(true)}
              onCompositionEnd={() => setComposing(false)}
              placeholder={placeholder}
              disabled={disabled}
              aria-autocomplete="list"
              aria-controls={menuOpen && entries.length > 0 ? menuId : undefined}
              aria-activedescendant={menuOpen && entries.length > 0 ? `${menuId}-${activeIndex}` : undefined}
            />
          </>
        ) : (
          <div className="prose g2-preview" role="tabpanel" aria-label="Markdown önizlemesi">
            {expanded.trim() ? <Markdown text={expanded} /> : <p className="muted">Önizlenecek bir şey yok.</p>}
          </div>
        )}
      </div>

      {pasteIds.length > 0 && (
        <div className="g2-chips">
          {pasteIds.map((id) => (
            <button
              key={id}
              type="button"
              className="g2-chip"
              onClick={() => {
                const next = expandPaste(draft, pastes, id)
                pastes.delete(id)
                setText(next, Math.min(caret, next.length))
              }}
              title="Tam metni taslağa aç"
            >
              {pasteLabel(pastes.get(id) ?? '', id).slice(1, -1)}
              <span className="g2-chip-action">aç</span>
            </button>
          ))}
        </div>
      )}

      {images.length > 0 && (
        <p className="g2-note" role="status">
          {OCR_NOTE}
        </p>
      )}
      {note && (
        <p className="g2-note" role="status">
          <span>{note}</span>
          <button type="button" className="action-btn g2-note-close" onClick={() => setNote(null)} aria-label="Bildirimi kapat">
            <X size={12} aria-hidden="true" />
          </button>
        </p>
      )}

      {/* §23.3 alt araç çubuğu üçe bölünür (≤ 8 kontrol): sol bağlam · orta model ve mod · sağ maliyet, token, Gönder */}
      <div className="composer-row kor-composer-bar">
        <div className="kor-composer-group" role="group" aria-label="Bağlam">
          <FolderMenu
            root={props.root}
            recents={props.recentWorkspaces}
            onOpen={props.onOpenFolder}
            onClear={props.onClearFolder}
            onPick={props.onPickRecentFolder}
          />
          <AttachMenu onPickFiles={props.onPickFiles} disabled={disabled} />
          {/* [j4] sesle görev (§10): yerel whisper.cpp, metin taslağa eklenir */}
          <VoiceButton draft={draft} onDraftChange={onDraftChange} disabled={disabled} />
        </div>
        <div className="kor-composer-group center" role="group" aria-label="Model ve mod">
          <ModelPicker value={props.model} models={props.models} error={props.modelsError} onChange={props.onModelChange} />
          <ModeMenu
            value={props.permissionMode}
            defaultMode={props.defaultPermissionMode}
            onChange={props.onPermissionModeChange}
            onMakeDefault={props.onMakeDefaultMode}
          />
        </div>
        <div className="kor-composer-group composer-right" role="group" aria-label="Maliyet ve gönder">
          {/* [g9] gerçek maliyet ("≈ $0.004"); tahmin yoksa boş */}
          <CostChip sessionId={props.sessionId} model={props.model} draftTokens={tokens} running={running} onAbort={props.onAbort} />
          <Tooltip text="Taslak ve dosya ekleri; yaklaşık 4 karakter = 1 token, görseller dahil değil">
            <span className="kor-composer-meta" tabIndex={0}>
              · {tokens.toLocaleString(uiLocale())} token
            </span>
          </Tooltip>
          {running ? (
            <>
              {bridge && canSend && (
                <Tooltip text="Sıraya ekle (Enter) · Ctrl+Enter: turu kesip şimdi gönder">
                  <button type="submit" className="action-btn" aria-label="Sıraya ekle">
                    <ListPlus size={16} aria-hidden="true" />
                  </button>
                </Tooltip>
              )}
              <Tooltip text="Durdur (Esc)">
                <button type="button" className="send-btn" onClick={() => void abort()} aria-label="Durdur">
                  <span className="stop-square" aria-hidden="true" />
                </button>
              </Tooltip>
            </>
          ) : (
            <Tooltip text="Gönder (Enter)">
              <button type="submit" className="send-btn" disabled={!canSend} aria-label="Gönder" data-kor-cmd="send">
                <ArrowUp size={16} aria-hidden="true" />
              </button>
            </Tooltip>
          )}
        </div>
      </div>

      {(attachments.length > 0 || images.length > 0) && (
        <AttachmentCards
          files={attachments}
          images={images}
          stats={stats}
          onRemoveFile={props.onRemoveAttachment}
          onRemoveImage={(id) => bridge?.setImages((prev) => prev.filter((i) => i.id !== id))}
          onMoveFile={(from, to) => bridge?.setAttachments((prev) => moveItem(prev, from, to))}
          onMoveImage={(from, to) => bridge?.setImages((prev) => moveItem(prev, from, to))}
        />
      )}
    </form>
    {parityRun && <ParityRunner command={parityRun.command} initialArgs={parityRun.rest} onClose={() => setParityRun(null)} />}
    </>
  )
}
