// cdesktop sohbet alanının sağ kenarındaki dikey eylem şeridi: ⋯ (menü), editörde aç (VS Code/Cursor), yolu kopyala,
// çalıştır (Terminal), önizleme (Değişiklikler), Git. Panel görünümleri 'panel:view' olayıyla seçilir.
import { Check, Copy, ExternalLink, FolderOpen, GitBranch, Monitor, MoreHorizontal, Play } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { EditorId } from '../ipc-g4.ts'
import { api, errorMessage } from './api.ts'
import type { PanelView } from './CommandPalette.tsx'
import { usePopover } from './use-popover.ts'
import vscodeSvg from './g9/brand-svg/vscode.svg'

const EDITOR_LABEL: Record<EditorId, string> = { code: 'VS Code', cursor: 'Cursor', windsurf: 'Windsurf', zed: 'Zed' }

export function FloatingRail({ root, onOpenPanel, onOpenFolder }: { root: string | null; onOpenPanel: (view: PanelView) => void; onOpenFolder: () => void }) {
  const menu = usePopover<HTMLDivElement>()
  const { data: editors } = useSWR(root ? 'g4:editors' : null, () => api.g4.detectEditors(), { revalidateOnFocus: false })
  const [copied, setCopied] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const editor = editors?.[0] ?? null
  const { data: settings } = useSWR('g7:settings', () => api.g7GetSettings(), { revalidateOnFocus: false })
  const preferred = settings?.editor.type ?? 'code'
  const preferredLabel = preferred === 'custom' ? 'Özel editör' : preferred === 'system' ? 'Varsayılan uygulama' : EDITOR_LABEL[preferred]
  const copyPath = () => {
    if (!root) return
    void navigator.clipboard.writeText(root).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }
  const openEditor = (id: EditorId | 'system' | 'preferred') => {
    if (!root) return
    void api.g4.openInEditor('.', null, id).catch((err: unknown) => {
      setNote(errorMessage(err))
      setTimeout(() => setNote(null), 3000)
    })
  }
  const Btn = ({ label, onClick, children, disabled }: { label: string; onClick: () => void; children: React.ReactNode; disabled?: boolean }) => (
    <button type="button" className="cdx-rail-btn" onClick={onClick} disabled={disabled} aria-label={label} title={label}>
      {children}
    </button>
  )
  return (
    <div className="cdx-rail" aria-label="Hızlı eylemler">
      <div className="popover-anchor" ref={menu.ref}>
        <button type="button" className="cdx-rail-btn" onClick={() => menu.setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={menu.open} aria-label="Daha fazla" title="Daha fazla">
          <MoreHorizontal size={15} aria-hidden="true" />
        </button>
        {menu.open && (
          <div className="menu cdx-rail-menu pop-in" role="menu">
            <button type="button" role="menuitem" className="menu-row" onClick={() => { menu.setOpen(false); onOpenFolder() }}><FolderOpen size={14} aria-hidden="true" /> Klasör aç…</button>
            {(['code', 'cursor', 'windsurf', 'zed'] as const).map((id) => (
              <button key={id} type="button" role="menuitem" className="menu-row" disabled={!editors?.includes(id)} onClick={() => { menu.setOpen(false); openEditor(id) }}>
                <ExternalLink size={14} aria-hidden="true" /> {EDITOR_LABEL[id]} ile aç
              </button>
            ))}
            <button type="button" role="menuitem" className="menu-row" disabled={!root} onClick={() => { menu.setOpen(false); openEditor('system') }}><FolderOpen size={14} aria-hidden="true" /> Dosya yöneticisinde göster</button>
          </div>
        )}
      </div>
      <Btn label={`${preferredLabel} ile aç (Ayarlar > Editör)`} onClick={() => openEditor('preferred')} disabled={!root && !editor}>
        <span className="cdx-rail-vscode" aria-hidden="true" dangerouslySetInnerHTML={{ __html: vscodeSvg }} />
      </Btn>
      <Btn label={copied ? 'Kopyalandı' : 'Klasör yolunu kopyala'} onClick={copyPath} disabled={!root}>
        {copied ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}
      </Btn>
      <Btn label="Çalıştır (Terminal)" onClick={() => onOpenPanel('terminal')}><Play size={15} aria-hidden="true" /></Btn>
      <Btn label="Web önizleme (dev sunucusu)" onClick={() => onOpenPanel('devpreview')}><Monitor size={15} aria-hidden="true" /></Btn>
      <Btn label="Git" onClick={() => onOpenPanel('git')}><GitBranch size={15} aria-hidden="true" /></Btn>
      {note && <p className="cdx-rail-note" role="status">{note}</p>}
    </div>
  )
}
