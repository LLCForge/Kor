// Composer ek menüsü: dosya/görsel ekle + "Görsel girdi" görünümü (ekran görüntüsü, diyagram, ses).
import { Paperclip, ScanEye } from 'lucide-react'
import { Tooltip } from './kor-ui/index.ts'
import { usePopover } from './use-popover.ts'
import { openView } from './view-catalog.ts'

export function AttachMenu({ onPickFiles, disabled }: { onPickFiles: () => void; disabled: boolean }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text="Ekle: dosya, görsel ya da görsel girdi (@ ile dosya, Ctrl+V ile görsel yapıştır)">
        <button type="button" className="action-btn" onClick={() => setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={open} aria-label="Ekle">
          <Paperclip size={16} aria-hidden="true" />
        </button>
      </Tooltip>
      {open && (
        <div className="menu menu-left pop-in kor-attach-menu" role="menu" aria-label="Ekle">
          <button
            type="button"
            role="menuitem"
            className="menu-row"
            disabled={disabled}
            onClick={() => {
              setOpen(false)
              onPickFiles()
            }}
          >
            <Paperclip size={16} aria-hidden="true" />
            <span className="truncate">Dosya veya görsel ekle</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="menu-row"
            onClick={() => {
              setOpen(false)
              openView('visual')
            }}
          >
            <ScanEye size={16} aria-hidden="true" />
            <span className="truncate">Görsel girdi</span>
          </button>
        </div>
      )}
    </div>
  )
}
