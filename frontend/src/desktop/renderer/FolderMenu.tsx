// Composer'daki çalışma klasörü seçici: "Klasör yok" (klasörsüz sohbet), son kullanılanlar ve "Klasör aç...".
// Klasörsüz sohbet, dosya/komut araçlarını güvenli varsayılan çalışma alanında çalıştırır (bkz. main.ts chatRoot).
import { Check, ChevronDown, Folder, FolderOpen } from 'lucide-react'
import { usePopover } from './use-popover.ts'
import { Tooltip } from './kor-ui/index.ts'
import { displayPath } from './display-path.ts'

interface FolderMenuProps {
  root: string | null
  recents: readonly string[]
  onOpen: () => void
  onClear: () => void
  onPick: (path: string) => void
}

const basename = (p: string): string => p.split(/[\\/]/).filter(Boolean).pop() ?? p

export function FolderMenu({ root, recents, onOpen, onClear, onPick }: FolderMenuProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const others = recents.filter((r) => r !== root)

  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text={root ? displayPath(root) : 'Klasör seçili değil — klasörsüz sohbet'}>
        <button
          type="button"
          className="mode-trigger folder-trigger"
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={open}
        >
          <Folder size={14} aria-hidden="true" />
          <span className="folder-trigger-label">{root ? basename(root) : 'Klasör yok'}</span>
          <ChevronDown size={14} className="dim" aria-hidden="true" />
        </button>
      </Tooltip>

      {open && (
        <div className="menu menu-left pop-in" role="menu" aria-label="Çalışma klasörü">
          <button
            type="button"
            role="menuitemradio"
            aria-checked={root === null}
            className="mode-item"
            onClick={() => {
              onClear()
              setOpen(false)
            }}
          >
            <span className="mode-main">
              <span className="mode-label">Klasör yok</span>
              <span className="mode-desc">Klasörsüz sohbet; dosyalar varsayılan çalışma alanında</span>
            </span>
            <span className="mode-check">{root === null && <Check size={16} aria-hidden="true" />}</span>
          </button>

          {root && (
            <button type="button" role="menuitemradio" aria-checked className="mode-item" title={displayPath(root)}>
              <span className="mode-main">
                <span className="mode-label">{basename(root)}</span>
                <span className="mode-desc folder-path">{displayPath(root)}</span>
              </span>
              <span className="mode-check">
                <Check size={16} aria-hidden="true" />
              </span>
            </button>
          )}

          {others.length > 0 && <p className="menu-heading">Son kullanılanlar</p>}
          {others.map((r) => (
            <button
              key={r}
              type="button"
              role="menuitem"
              className="mode-item"
              title={displayPath(r)}
              onClick={() => {
                onPick(r)
                setOpen(false)
              }}
            >
              <span className="mode-main">
                <span className="mode-label">{basename(r)}</span>
                <span className="mode-desc folder-path">{displayPath(r)}</span>
              </span>
            </button>
          ))}

          <button
            type="button"
            className="menu-footer-btn"
            onClick={() => {
              onOpen()
              setOpen(false)
            }}
          >
            <FolderOpen size={14} aria-hidden="true" /> Klasör aç...
          </button>
        </div>
      )}
    </div>
  )
}
