// cdesktop komut paleti (Ctrl/⌘+K): "Komut yaz ya da ara…", gruplu satırlar (Eylemler / Görünüm / Genel),
// sağda akor ipuçları (G N), alt palet ("‹ Geri" satırı). Esc kapatır, ↑↓ gezer, Enter çalıştırır.
import { ArrowLeft, Columns2, Compass, FileDown, FileText, FolderOpen, GitBranch, Globe, Keyboard, LayoutPanelLeft, MessageSquare, PanelRight, Plus, ScrollText, Search, Settings, SquareTerminal, Stethoscope, StickyNote, Copy } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import useSWR from 'swr'
import type { SessionMeta } from '../ipc.ts'
import { Modal } from './NameDialog.tsx'
import { relativeTime } from './session-meta.ts'
import { api, errorMessage } from './api.ts'

import { createElement } from 'react'
import { PANELS, findPanel } from './panels/registry.ts'
import { SETTINGS_VIEWS } from './view-catalog.ts'
import { Bell, ListChecks, PictureInPicture2 } from 'lucide-react'
import { shellui } from './shellui/bridge.ts'
import { OPEN_BELL_EVENT } from './shellui/NotificationCenter.tsx'
import { OPEN_QUEUE_EVENT } from './shellui/TitleItems.tsx'
import { PARITY_COMMANDS, type ParityCommand } from './parity-commands.gen.ts'
import { panelIdsFor } from './parity-palette.ts'
import { ParityRunner } from './ParityRunner.tsx'

export type BuiltinPanelView = 'changes' | 'preview' | 'git' | 'terminal' | 'notes' | 'logs' | 'diagnostics' | 'devpreview' | 'browser'
/** Yerleşik görünümler + panels/registry.ts kimlikleri */
export type PanelView = BuiltinPanelView | (string & {})

export interface PaletteActions {
  newChat: () => void
  openFolder: () => void
  copyPath: () => void
  openPanel: (view: PanelView) => void
  openSettings: () => void
  openSession: (id: string) => void
  showShortcuts: () => void
  // [cdx] tam akor seti
  toggleSidebar: () => void
  archiveCurrent: () => void
  pinCurrent: () => void
  deleteCurrent: () => void
  duplicateCurrent: () => void
  renameCurrent: () => void
  nextSession: () => void
  prevSession: () => void
  gitPush: () => void
  gitMerge: () => void
  gitRebase: () => void
  gitPr: () => void
  exportCurrent: () => void
}

interface Item {
  id: string
  label: string
  icon: ReactNode
  group: string
  hint?: string
  keywords?: string
  run: () => void
  /** true: alt palet açar (kapatmaz) */
  sub?: boolean
}

/** cdesktop kısayol seti (akorlar: ilk tuş, sonra 1,2 sn içinde ikinci tuş; yazı alanı odakta değilken). */
export const SHORTCUT_GROUPS: ReadonlyArray<{ title: string; items: ReadonlyArray<{ keys: string; label: string }> }> = [
  { title: 'Hızlı eylemler', items: [{ keys: '?', label: 'Klavye kısayolları' }, { keys: 'Esc', label: 'Paleti / menüyü kapat' }, { keys: 'Ctrl/⌘ + K', label: 'Komut paleti' }, { keys: 'Ctrl/⌘ + ,', label: 'Ayarlar' }, { keys: 'Ctrl/⌘ + Enter', label: 'Mesajı gönder' }] },
  { title: 'Gezinme', items: [{ keys: 'J', label: 'Sonraki sohbet' }, { keys: 'K', label: 'Önceki sohbet' }, { keys: 'G N', label: 'Yeni sohbet' }, { keys: 'G O', label: 'Klasör aç' }, { keys: 'G S', label: 'Sohbete git…' }] },
  { title: 'Sohbet', items: [{ keys: 'W R', label: 'Yeniden adlandır' }, { keys: 'W D', label: 'Çoğalt' }, { keys: 'W P', label: 'Sabitle / kaldır' }, { keys: 'W A', label: 'Arşivle / çıkar' }, { keys: 'W X', label: 'Sil' }, { keys: 'W E', label: 'Markdown olarak dışa aktar' }] },
  { title: 'Görünüm', items: [{ keys: 'G V', label: 'Görünüm seçenekleri' }, { keys: 'V C', label: 'Değişiklikler' }, { keys: 'V G', label: 'Git' }, { keys: 'V T', label: 'Terminal' }, { keys: 'V N', label: 'Notlar' }, { keys: 'V L', label: 'Günlükler' }, { keys: 'V P', label: 'Önizleme' }, { keys: 'V W', label: 'Web önizleme' }, { keys: 'V B', label: 'Tarayıcı' }, { keys: 'V S', label: 'Kenar çubuğunu aç/kapat' }] },
  { title: 'Git', items: [{ keys: 'X P', label: 'Pull request aç' }, { keys: 'X U', label: 'Push' }, { keys: 'X M', label: 'Merge' }, { keys: 'X R', label: 'Rebase' }] },
  { title: 'Kopyala', items: [{ keys: 'Y P', label: 'Klasör yolunu kopyala' }] },
]
export const SHORTCUTS = SHORTCUT_GROUPS.flatMap((g) => g.items)

export function CommandPalette({ actions, sessions }: { actions: PaletteActions; sessions: readonly SessionMeta[] }) {
  const [open, setOpen] = useState(false)
  const [page, setPage] = useState<'root' | 'views' | 'sessions' | 'templates' | 'parity'>('root')
  // Tüm komutlar (ADR 0031): panelsiz köprü yöntemi seçilince açılan genel çalıştırıcı.
  const [runner, setRunner] = useState<ParityCommand | null>(null)
  const [templateError, setTemplateError] = useState<string | null>(null)
  // Görev şablonları (§15.11): yalnızca alt palet açıkken yüklenir.
  const { data: templateList } = useSWR(open && page === 'templates' ? 'templates' : null, () => api.templates.list())
  const [query, setQuery] = useState('')
  const [index, setIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const chord = useRef<{ key: string; at: number } | null>(null)

  // Küresel kısayollar: Ctrl/⌘+K palet; G N / G O / G V / G S akorları (yazı alanı odakta değilken); ? kısayollar.
  useEffect(() => {
    const typing = () => {
      const el = document.activeElement
      return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || (el as HTMLElement | null)?.isContentEditable === true
    }
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setOpen((o) => !o)
        setPage('root')
        setQuery('')
        return
      }
      if ((e.ctrlKey || e.metaKey) && e.key === ',') {
        e.preventDefault()
        actions.openSettings()
        return
      }
      // §21.1: Ctrl+P (yazdır) ve Ctrl+S (farklı kaydet) tarayıcıya değil, Kor'un oturum dışa aktarmasına gider.
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (e.key.toLowerCase() === 'p' || e.key.toLowerCase() === 's')) {
        e.preventDefault()
        actions.exportCurrent()
        return
      }
      if (open || typing() || e.ctrlKey || e.metaKey || e.altKey) return
      if (e.key === '?') {
        e.preventDefault()
        actions.showShortcuts()
        return
      }
      const now = Date.now()
      const prev = chord.current
      const k = e.key.toLowerCase()
      // Tek tuş: J/K sohbetler arasında gezinir.
      if (!prev && k === 'j') { e.preventDefault(); actions.nextSession(); return }
      if (!prev && k === 'k') { e.preventDefault(); actions.prevSession(); return }
      if (prev && now - prev.at < 1200) {
        chord.current = null
        const run = (fn: () => void) => { e.preventDefault(); fn() }
        const table: Record<string, Record<string, () => void>> = {
          g: { n: actions.newChat, o: actions.openFolder, v: () => { setOpen(true); setPage('views'); setQuery('') }, s: () => { setOpen(true); setPage('sessions'); setQuery('') } },
          v: { c: () => actions.openPanel('changes'), g: () => actions.openPanel('git'), t: () => actions.openPanel('terminal'), n: () => actions.openPanel('notes'), l: () => actions.openPanel('logs'), p: () => actions.openPanel('preview'), w: () => actions.openPanel('devpreview'), b: () => actions.openPanel('browser'), s: actions.toggleSidebar },
          w: { r: actions.renameCurrent, d: actions.duplicateCurrent, p: actions.pinCurrent, a: actions.archiveCurrent, x: actions.deleteCurrent, e: actions.exportCurrent },
          x: { p: actions.gitPr, u: actions.gitPush, m: actions.gitMerge, r: actions.gitRebase },
          y: { p: actions.copyPath },
        }
        const fn = table[prev.key]?.[k]
        if (fn) run(fn)
        return
      }
      chord.current = ['g', 'v', 'w', 'x', 'y'].includes(k) ? { key: k, at: now } : null
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, actions])

  // [osint] global kısayol / Dock menüsü komut paletini açar (ADR 0052)
  useEffect(() => {
    // detail 'views': panel başlığındaki "Tüm görünümler…" paleti Görünüm sayfasında açar.
    const onOpen = (e: Event) => { setOpen(true); setPage((e as CustomEvent<unknown>).detail === 'views' ? 'views' : 'root'); setQuery(''); setIndex(0) }
    window.addEventListener('osint:palette', onOpen)
    return () => window.removeEventListener('osint:palette', onOpen)
  }, [])

  useEffect(() => {
    if (open) requestAnimationFrame(() => inputRef.current?.focus())
  }, [open, page])

  const close = () => { setOpen(false); setQuery(''); setPage('root'); setTemplateError(null) }
  const sub = (p: 'views' | 'sessions' | 'templates' | 'parity') => { setPage(p); setQuery(''); setIndex(0); setTemplateError(null) }

  const items = useMemo<Item[]>(() => {
    if (page === 'views') {
      const views: Array<[PanelView, string, ReactNode]> = [
        ['changes', 'Değişiklikler', <Columns2 size={14} />],
        ['preview', 'Önizleme', <LayoutPanelLeft size={14} />],
        ['git', 'Git', <GitBranch size={14} />],
        ['terminal', 'Terminal', <SquareTerminal size={14} />],
        ['notes', 'Notlar', <StickyNote size={14} />],
        ['logs', 'Günlükler', <ScrollText size={14} />],
        ['devpreview', 'Web önizleme', <Globe size={14} />],
        ['browser', 'Tarayıcı', <Compass size={14} />],
        ['diagnostics', 'Tanılama', <Stethoscope size={14} />],
        ...PANELS.map((p): [PanelView, string, ReactNode] => [p.id, p.label, createElement(p.icon, { size: 14 })]),
      ]
      return [
        { id: 'back', label: 'Geri', icon: <ArrowLeft size={14} />, group: '', run: () => sub('views'), sub: true },
        // Ayarlar'da açılanlar ayrı grupta, sonda: ok tuşları görünen sırayı izlesin.
        ...views.filter(([id]) => !SETTINGS_VIEWS[id]).concat(views.filter(([id]) => SETTINGS_VIEWS[id])).map(([id, label, icon]) => ({ id: `view:${id}`, label, icon, group: SETTINGS_VIEWS[id] ? 'Ayarlar' : 'Görünüm', run: () => actions.openPanel(id) })),
      ]
    }
    if (page === 'templates') {
      // Seçilen şablon varsayılanlarla taslağa yazılır; zorunlu girdiler `<ad>` olarak kalır, kullanıcı düzenleyip gönderir.
      const pickTemplate = (name: string) => {
        api.templates
          .draft(name)
          .then((r) => {
            window.dispatchEvent(new CustomEvent('cdx:draft', { detail: r.prompt }))
            setOpen(false); setQuery(''); setPage('root')
          })
          .catch((err: unknown) => setTemplateError(errorMessage(err)))
      }
      const sourceLabel = { project: 'proje', personal: 'kişisel', plugin: 'eklenti', builtin: 'yerleşik' } as const
      return [
        { id: 'back', label: 'Geri', icon: <ArrowLeft size={14} />, group: '', run: () => sub('templates'), sub: true },
        ...(templateList?.templates ?? []).map((t) => ({ id: `t:${t.name}`, label: `${t.name} · ${t.description}`, icon: <FileText size={14} />, group: 'Görev şablonları', hint: sourceLabel[t.source], keywords: t.inputs.map((i) => i.name).join(' '), run: () => pickTemplate(t.name), sub: true })),
      ]
    }
    if (page === 'parity') {
      // parity.toml'daki her masaüstü köprü yöntemi: paneli varsa panel, yoksa genel çalıştırıcı.
      return [
        { id: 'back', label: 'Geri', icon: <ArrowLeft size={14} />, group: '', run: () => sub('parity'), sub: true },
        ...PARITY_COMMANDS.map((c, i) => {
          const panel = panelIdsFor(c.method.split('.')[0]!).map(findPanel).find((p) => p !== undefined)
          return {
            id: `p:${i}`,
            label: c.name,
            icon: panel ? createElement(panel.icon, { size: 14 }) : <SquareTerminal size={14} />,
            group: 'Tüm komutlar',
            hint: panel ? panel.label : undefined,
            keywords: `${c.cli} ${c.method}`,
            run: panel ? () => actions.openPanel(panel.id) : () => setRunner(c),
          }
        }),
      ]
    }
    if (page === 'sessions') {
      return [
        { id: 'back', label: 'Geri', icon: <ArrowLeft size={14} />, group: '', run: () => sub('sessions'), sub: true },
        ...sessions.filter((s) => !s.archived).slice(0, 50).map((s) => ({ id: `s:${s.id}`, label: s.title, icon: <MessageSquare size={14} />, group: 'Sohbetler', hint: relativeTime(s.updatedAt), keywords: s.recap ?? '', run: () => actions.openSession(s.id) })),
      ]
    }
    return [
      { id: 'new', label: 'Yeni sohbet', icon: <Plus size={14} />, group: 'Eylemler', hint: 'G N', run: actions.newChat },
      { id: 'folder', label: 'Klasör aç', icon: <FolderOpen size={14} />, group: 'Eylemler', hint: 'G O', run: actions.openFolder },
      { id: 'copy', label: 'Klasör yolunu kopyala', icon: <Copy size={14} />, group: 'Eylemler', run: actions.copyPath },
      { id: 'export', label: 'Sohbeti Markdown olarak dışa aktar', icon: <FileDown size={14} />, group: 'Eylemler', hint: 'W E', run: actions.exportCurrent },
      { id: 'sessions', label: 'Sohbete git…', icon: <MessageSquare size={14} />, group: 'Eylemler', hint: 'G S', run: () => sub('sessions'), sub: true },
      { id: 'templates', label: 'Görev şablonları…', icon: <FileText size={14} />, group: 'Eylemler', run: () => sub('templates'), sub: true },
      { id: 'parity', label: 'Tüm komutlar…', icon: <ListChecks size={14} />, group: 'Eylemler', keywords: 'kor cli komut parite', run: () => sub('parity'), sub: true },
      { id: 'views', label: 'Görünüm seçenekleri…', icon: <PanelRight size={14} />, group: 'Görünüm', hint: 'G V', run: () => sub('views'), sub: true },
      { id: 'git', label: 'Git paneli', icon: <GitBranch size={14} />, group: 'Görünüm', run: () => actions.openPanel('git') },
      { id: 'terminal', label: 'Terminal', icon: <SquareTerminal size={14} />, group: 'Görünüm', run: () => actions.openPanel('terminal') },
      { id: 'devpreview', label: 'Web önizleme', icon: <Globe size={14} />, group: 'Görünüm', hint: 'V W', run: () => actions.openPanel('devpreview') },
      { id: 'browser', label: 'Tarayıcı (ajan)', icon: <Compass size={14} />, group: 'Görünüm', hint: 'V B', run: () => actions.openPanel('browser') },
      // ADR 0051: durum penceresi, bildirim merkezi, onay kuyruğu
      { id: 'pip', label: 'Durum penceresini aç/kapat', icon: <PictureInPicture2 size={14} />, group: 'Görünüm', keywords: 'pip her zaman üstte mini durum', run: () => void shellui.pip() },
      { id: 'bell', label: 'Bildirimler', icon: <Bell size={14} />, group: 'Görünüm', keywords: 'zil bildirim merkezi notification', run: () => window.dispatchEvent(new Event(OPEN_BELL_EVENT)) },
      { id: 'queue', label: 'Onay kuyruğu', icon: <ListChecks size={14} />, group: 'Görünüm', keywords: 'onay approve deny kuyruk bekleyen', run: () => window.dispatchEvent(new Event(OPEN_QUEUE_EVENT)) },
      { id: 'settings', label: 'Ayarlar', icon: <Settings size={14} />, group: 'Genel', hint: 'Ctrl ,', run: actions.openSettings },
      { id: 'keys', label: 'Klavye kısayolları', icon: <Keyboard size={14} />, group: 'Genel', hint: '?', run: actions.showShortcuts },
    ]
  }, [page, sessions, actions, templateList])

  const visible = useMemo(() => {
    const q = query.trim().toLocaleLowerCase('tr')
    if (!q) return items
    return items.filter((i) => i.id === 'back' || `${i.label} ${i.keywords ?? ''} ${i.group}`.toLocaleLowerCase('tr').includes(q))
  }, [items, query])

  useEffect(() => setIndex(0), [visible.length, page])
  if (!open) return runner ? <ParityRunner command={runner} onClose={() => setRunner(null)} /> : null

  const activate = (item: Item) => {
    if (item.id === 'back') { setPage('root'); setQuery(''); return }
    item.run()
    if (!item.sub) close()
  }
  const groups: string[] = []
  for (const i of visible) if (i.group && !groups.includes(i.group)) groups.push(i.group)

  return (
    <Modal onClose={close} labelledBy="cdx-palette-title">
      <div className="cdx-palette" role="dialog" aria-label="Komut paleti">
        <h2 id="cdx-palette-title" className="sr-only">Komut paleti</h2>
        <label className="cdx-palette-input" data-kor-focus-host="">
          <Search size={15} aria-hidden="true" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={page === 'views' ? 'Görünüm seç…' : page === 'sessions' ? 'Sohbet ara…' : page === 'templates' ? 'Şablon ara…' : page === 'parity' ? 'Komut ya da kor alt komutu ara…' : 'Komut yaz ya da ara…'}
            aria-label="Komut ara"
            spellCheck={false}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown') { e.preventDefault(); setIndex((i) => Math.min(i + 1, visible.length - 1)) }
              else if (e.key === 'ArrowUp') { e.preventDefault(); setIndex((i) => Math.max(i - 1, 0)) }
              else if (e.key === 'Enter') { e.preventDefault(); const it = visible[index]; if (it) activate(it) }
              else if (e.key === 'Backspace' && !query && page !== 'root') { e.preventDefault(); setPage('root') }
            }}
          />
        </label>
        <div className="cdx-palette-list" role="listbox">
          {visible.filter((i) => i.id === 'back').map((i) => (
            <button key={i.id} type="button" role="option" aria-selected={visible.indexOf(i) === index} className={`cdx-palette-row${visible.indexOf(i) === index ? ' active' : ''}`} onClick={() => activate(i)} onMouseEnter={() => setIndex(visible.indexOf(i))}>
              {i.icon}<span className="truncate">{i.label}</span>
            </button>
          ))}
          {groups.map((g) => (
            <div key={g} className="cdx-palette-group">
              <p className="cdx-palette-heading">{g}</p>
              {visible.filter((i) => i.group === g).map((i) => {
                const idx = visible.indexOf(i)
                return (
                  <button key={i.id} type="button" role="option" aria-selected={idx === index} className={`cdx-palette-row${idx === index ? ' active' : ''}`} onClick={() => activate(i)} onMouseEnter={() => setIndex(idx)}>
                    {i.icon}
                    <span className="truncate">{i.label}</span>
                    {i.hint && <kbd className="cdx-palette-hint">{i.hint}</kbd>}
                  </button>
                )
              })}
            </div>
          ))}
          {templateError && <p className="cdx-palette-empty" role="alert">{templateError}</p>}
          {visible.length === 0 && <p className="cdx-palette-empty">“{query}” ile eşleşen komut yok.</p>}
        </div>
      </div>
    </Modal>
  )
}

/** cdesktop klavye kısayolları penceresi. */
export function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal onClose={onClose} labelledBy="cdx-shortcuts-title">
      <div className="dialog-body cdx-shortcuts">
        <h2 id="cdx-shortcuts-title" className="dialog-title">Klavye kısayolları</h2>
        <p className="g7-note">Akorlar: ilk tuş, ardından 1,2 saniye içinde ikinci tuş (yazı alanı odakta değilken).</p>
        <div className="cdx-shortcut-groups">
          {/* İki sabit sütun (grid satır hizası boşluk bırakıyor, column-count ise taşıyordu): sol 0,2,4… sağ 1,3,5… */}
          {[0, 1].map((col) => (
            <div key={col} className="cdx-shortcut-col">
              {SHORTCUT_GROUPS.filter((_g, i) => i % 2 === col).map((g) => (
                <section key={g.title} className="cdx-shortcut-group">
                  <h3>{g.title}</h3>
                  <ul className="cdx-shortcut-list">
                    {g.items.map((s) => (
                      <li key={s.keys}><span>{s.label}</span><kbd>{s.keys}</kbd></li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          ))}
        </div>
        <div className="dialog-actions"><span /><div className="dialog-actions-right"><button type="button" className="btn primary" onClick={onClose}>Kapat</button></div></div>
      </div>
    </Modal>
  )
}
