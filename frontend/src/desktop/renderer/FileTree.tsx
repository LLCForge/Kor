import {
  ChevronDown,
  ChevronRight,
  ClipboardCopy,
  EyeOff,
  File,
  FilePlus,
  Folder,
  FolderOpen,
  FolderPlus,
  Paperclip,
  Pencil,
  Search,
  Trash2,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { G4FileEntry } from '../ipc-g4.ts'
import { api, errorMessage } from './api.ts'
import { buildTree, fuzzyFilter, type FilterNode } from './fuzzy.ts'
import { requestAttach } from './g4-shared.ts'
import { NameDialog } from './NameDialog.tsx'
import { confirmAction } from './ConfirmDialog.tsx'
import { usePersistentChoice } from './persisted.ts'

export const dirKey = (dir: string, respectGitignore = false) => ['dir', dir, respectGitignore] as const
const isTreeKey = (key: unknown) => Array.isArray(key) && (key[0] === 'dir' || key[0] === 'g4:allFiles')

interface TreeEntry {
  name: string
  path: string
  isDir: boolean
  sensitive: boolean
}

interface MenuState {
  entry: TreeEntry
  x: number
  y: number
}

type DialogState = { kind: 'newFile' | 'newDir' | 'rename'; entry: TreeEntry } | null

interface TreeContext {
  selected: string | null
  changed: ReadonlySet<string>
  respectGitignore: boolean
  onOpenFile: (path: string) => void
  onMenu: (entry: TreeEntry, x: number, y: number) => void
}

const parentOf = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '')
const indentFor = (depth: number) => ({ paddingLeft: 8 + depth * 14 })

function Row({ entry, depth, open, onToggle, ctx }: { entry: TreeEntry; depth: number; open: boolean; onToggle: () => void; ctx: TreeContext }) {
  const isSelected = !entry.isDir && ctx.selected === entry.path
  const openMenu = (e: MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    ctx.onMenu(entry, e.clientX, e.clientY)
  }
  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      const rect = e.currentTarget.getBoundingClientRect()
      ctx.onMenu(entry, rect.left + 24, rect.bottom)
    }
  }
  return (
    <button
      type="button"
      className={`tree-row${isSelected ? ' selected' : ''}`}
      style={indentFor(depth)}
      onClick={() => (entry.isDir ? onToggle() : ctx.onOpenFile(entry.path))}
      onContextMenu={openMenu}
      onKeyDown={onKeyDown}
      title={entry.path}
    >
      {entry.isDir ? (
        open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />
      ) : (
        <span className="tree-chevron" aria-hidden="true" />
      )}
      {entry.isDir ? (
        open ? <FolderOpen size={14} aria-hidden="true" /> : <Folder size={14} aria-hidden="true" />
      ) : (
        <File size={14} aria-hidden="true" />
      )}
      <span className="tree-name">{entry.name}</span>
      {!entry.isDir && ctx.changed.has(entry.path) && <span className="tree-badge" aria-label="değişti">M</span>}
    </button>
  )
}

// ---------- Tembel (klasör açıldıkça yüklenen) ağaç ----------

function Directory({ dir, depth, ctx }: { dir: string; depth: number; ctx: TreeContext }) {
  const { data, error, isLoading } = useSWR(dirKey(dir, ctx.respectGitignore), () => api.g4.listDir(dir, ctx.respectGitignore))
  if (isLoading) return <div className="tree-status" style={{ paddingLeft: 12 + depth * 14 }}>Yükleniyor...</div>
  if (error) return <div className="tree-status error" style={{ paddingLeft: 12 + depth * 14 }}>{errorMessage(error)}</div>
  if (!data || data.length === 0) return <div className="tree-status" style={{ paddingLeft: 12 + depth * 14 }}>Boş</div>
  return (
    <ul role="group" className="tree-list">
      {data.map((entry) => (
        <LazyNode key={entry.path} entry={entry} depth={depth} ctx={ctx} />
      ))}
    </ul>
  )
}

function LazyNode({ entry, depth, ctx }: { entry: G4FileEntry; depth: number; ctx: TreeContext }) {
  const [open, setOpen] = useState(false)
  return (
    <li role="treeitem" aria-expanded={entry.isDir ? open : undefined} aria-selected={!entry.isDir && ctx.selected === entry.path}>
      <Row entry={entry} depth={depth} open={open} onToggle={() => setOpen((o) => !o)} ctx={ctx} />
      {entry.isDir && open && <Directory dir={entry.path} depth={depth + 1} ctx={ctx} />}
    </li>
  )
}

// ---------- Arama sonuçları (eşleşen klasörler açık) ----------

function FilteredNode({ node, depth, ctx }: { node: FilterNode; depth: number; ctx: TreeContext }) {
  const [open, setOpen] = useState(true)
  const entry: TreeEntry = { name: node.name, path: node.path, isDir: node.isDir, sensitive: false }
  return (
    <li role="treeitem" aria-expanded={node.isDir ? open : undefined} aria-selected={!node.isDir && ctx.selected === node.path}>
      <Row entry={entry} depth={depth} open={open} onToggle={() => setOpen((o) => !o)} ctx={ctx} />
      {node.isDir && open && (
        <ul role="group" className="tree-list">
          {node.children.map((child) => (
            <FilteredNode key={`${child.isDir ? 'd' : 'f'}:${child.path}`} node={child} depth={depth + 1} ctx={ctx} />
          ))}
        </ul>
      )}
    </li>
  )
}

function SearchResults({ query, ctx }: { query: string; ctx: TreeContext }) {
  const { data, error, isLoading } = useSWR(['g4:allFiles', ctx.respectGitignore], () => api.g4.listAllFiles(ctx.respectGitignore))
  const matches = useMemo(() => (data ? fuzzyFilter(data.files, query) : []), [data, query])
  const nodes = useMemo(() => buildTree(matches), [matches])
  if (isLoading) return <div className="tree-status">Dosyalar taranıyor...</div>
  if (error) return <div className="tree-status error">{errorMessage(error)}</div>
  if (nodes.length === 0) return <div className="tree-status">"{query}" ile eşleşen dosya yok.</div>
  return (
    <>
      <ul role="group" className="tree-list">
        {nodes.map((node) => (
          <FilteredNode key={`${node.isDir ? 'd' : 'f'}:${node.path}`} node={node} depth={0} ctx={ctx} />
        ))}
      </ul>
      <p className="tree-status" role="status">
        {matches.length} sonuç{data?.truncated ? ' (klasör çok büyük, tarama kısmi)' : ''}
      </p>
    </>
  )
}

// ---------- Sağ tık menüsü ----------

type MenuAction = 'newFile' | 'newDir' | 'rename' | 'copy' | 'attach' | 'trash'

interface TreeMenuProps {
  menu: MenuState
  onClose: () => void
  onAction: (action: MenuAction) => void
}

function TreeMenu({ menu, onClose, onAction }: TreeMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  const { entry } = menu
  const isRoot = entry.path === ''
  const blocked = entry.sensitive

  // Üst bileşen her yeniden çizildiğinde odak sıfırlanmasın: etki bir kez kurulur, güncel onClose ref'ten okunur.
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose
  useEffect(() => {
    ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
    const onPointer = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onCloseRef.current()
    }
    document.addEventListener('pointerdown', onPointer)
    return () => document.removeEventListener('pointerdown', onPointer)
  }, [])

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      onClose()
      return
    }
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])]
    const index = items.indexOf(document.activeElement as HTMLButtonElement)
    const next = items[(index + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]
    next?.focus()
  }

  // Menü pencere dışına taşmasın.
  const style = { left: Math.min(menu.x, window.innerWidth - 230), top: Math.min(menu.y, window.innerHeight - 290) }
  const item = (action: MenuAction, label: string, Icon: typeof File, disabled = false, danger = false) => (
    <button type="button" role="menuitem" className={`menu-row${danger ? ' danger' : ''}`} onClick={() => onAction(action)} disabled={disabled}>
      <Icon size={14} aria-hidden="true" /> {label}
    </button>
  )

  return (
    <div ref={ref} className="menu g4-context-menu pop-in" role="menu" aria-label={`${entry.name || 'Kök klasör'} işlemleri`} style={style} onKeyDown={onKeyDown}>
      {item('newFile', 'Yeni dosya', FilePlus)}
      {item('newDir', 'Yeni klasör', FolderPlus)}
      {item('rename', 'Yeniden adlandır', Pencil, isRoot || blocked)}
      {item('copy', 'Yolu kopyala', ClipboardCopy)}
      {!entry.isDir && item('attach', 'Sohbete ekle', Paperclip, blocked)}
      {item('trash', 'Çöp kutusuna taşı', Trash2, isRoot || blocked, true)}
      {blocked && <p className="menu-note">Hassas dosya: düzenleme işlemleri kapalı.</p>}
    </div>
  )
}

// ---------- Ağaç ----------

interface FileTreeProps {
  rootName: string
  selected: string | null
  changed: ReadonlySet<string>
  onOpenFile: (path: string) => void
}

export function FileTree({ rootName, selected, changed, onOpenFile }: FileTreeProps) {
  const { mutate } = useSWRConfig()
  const [query, setQuery] = useState('')
  const [gitignore, setGitignore] = usePersistentChoice('g4.gitignore', ['on', 'off'] as const, 'on')
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [dialog, setDialog] = useState<DialogState>(null)
  const [opError, setOpError] = useState<string | null>(null)
  const [copied, setCopied] = useState<string | null>(null)
  const respectGitignore = gitignore === 'on'

  const ctx: TreeContext = {
    selected,
    changed,
    respectGitignore,
    onOpenFile,
    onMenu: (entry, x, y) => setMenu({ entry, x, y }),
  }
  const rootEntry: TreeEntry = { name: rootName, path: '', isDir: true, sensitive: false }
  const refresh = () => mutate(isTreeKey)

  const act = async (action: MenuAction, entry: TreeEntry) => {
    setMenu(null)
    setOpError(null)
    try {
      if (action === 'newFile' || action === 'newDir' || action === 'rename') {
        setDialog({ kind: action, entry })
      } else if (action === 'copy') {
        await navigator.clipboard.writeText(entry.path || '.')
        setCopied(entry.path || '.')
      } else if (action === 'attach') {
        requestAttach(entry.path)
      } else if (action === 'trash') {
        const what = entry.isDir ? `"${entry.name}" klasörü ve tüm içeriği` : `"${entry.name}"`
        const ok = await confirmAction({
          title: 'Çöp kutusuna taşınsın mı?',
          message: `${what} çöp kutusuna taşınacak. Gerekirse çöp kutusundan geri alabilirsin.`,
          confirmLabel: 'Çöpe taşı',
          danger: true,
        })
        if (!ok) return
        await api.g4.trash(entry.path)
        await refresh()
      }
    } catch (err) {
      setOpError(errorMessage(err))
    }
  }

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(null), 2000)
    return () => clearTimeout(timer)
  }, [copied])

  const targetDir = (entry: TreeEntry) => (entry.isDir ? entry.path : parentOf(entry.path))

  return (
    <nav className="file-tree" aria-label={`${rootName} dosyaları`}>
      <div className="g4-tree-head">
        <h2 className="rail-heading">Dosyalar</h2>
        <button
          type="button"
          className={`action-btn${respectGitignore ? ' active' : ''}`}
          onClick={() => setGitignore(respectGitignore ? 'off' : 'on')}
          aria-pressed={respectGitignore}
          aria-label=".gitignore'daki dosyaları gizle"
          title={respectGitignore ? '.gitignore uygulanıyor (yok sayılan dosyalar gizli)' : '.gitignore uygulanmıyor (tüm dosyalar görünür)'}
        >
          <EyeOff size={14} aria-hidden="true" />
        </button>
        <button type="button" className="action-btn" onClick={(e) => setMenu({ entry: rootEntry, x: e.clientX, y: e.clientY })} aria-label="Kök klasörde yeni öğe" title="Yeni dosya veya klasör">
          <FilePlus size={14} aria-hidden="true" />
        </button>
      </div>
      <div className="g4-tree-search">
        <Search size={13} aria-hidden="true" />
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('')
          }}
          placeholder="Dosya ara (bulanık)"
          aria-label="Dosyalarda ara"
          spellCheck={false}
        />
        {query && (
          <button type="button" className="action-btn" onClick={() => setQuery('')} aria-label="Aramayı temizle">
            <X size={12} aria-hidden="true" />
          </button>
        )}
      </div>
      {opError && (
        <p className="tree-status error" role="alert">
          {opError}
        </p>
      )}
      {copied && (
        <p className="tree-status" role="status">
          Kopyalandı: {copied}
        </p>
      )}
      <div role="tree" className="tree-scroll" onContextMenu={(e) => {
        e.preventDefault()
        setMenu({ entry: rootEntry, x: e.clientX, y: e.clientY })
      }}>
        {query.trim() ? <SearchResults query={query} ctx={ctx} /> : <Directory dir="" depth={0} ctx={ctx} />}
      </div>

      {menu && <TreeMenu menu={menu} onClose={() => setMenu(null)} onAction={(action) => void act(action, menu.entry)} />}

      {dialog?.kind === 'rename' && (
        <NameDialog
          title="Yeniden adlandır"
          description={dialog.entry.path}
          initial={dialog.entry.name}
          submitLabel="Yeniden adlandır"
          onSubmit={async (name) => {
            await api.g4.rename(dialog.entry.path, name)
            await refresh()
          }}
          onClose={() => setDialog(null)}
        />
      )}
      {(dialog?.kind === 'newFile' || dialog?.kind === 'newDir') && (
        <NameDialog
          title={dialog.kind === 'newFile' ? 'Yeni dosya' : 'Yeni klasör'}
          description={`Konum: ${targetDir(dialog.entry) || rootName}`}
          initial=""
          submitLabel="Oluştur"
          onSubmit={async (name) => {
            const dir = targetDir(dialog.entry)
            const created = dialog.kind === 'newFile' ? await api.g4.createFile(dir, name) : await api.g4.createDir(dir, name)
            await refresh()
            if (dialog.kind === 'newFile') onOpenFile(created)
          }}
          onClose={() => setDialog(null)}
        />
      )}
    </nav>
  )
}
