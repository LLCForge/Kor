import { FileText, Folder, X } from 'lucide-react'
import { useState, type DragEvent, type KeyboardEvent, type ReactNode } from 'react'
import type { G2FileStat } from '../../ipc-g2.ts'
import { formatBytes } from './composer-utils.ts'
import type { PendingImage } from './images.ts'

const DRAG_TYPE = 'application/x-venice-attachment'
type Kind = 'file' | 'image'

interface CardProps {
  kind: Kind
  index: number
  count: number
  label: string
  title: string
  meta: string
  visual: ReactNode
  /** Görseller için: ad/boyut metni yerine resmi tam kaplayan yuvarlak karo. */
  tile?: boolean
  onRemove: () => void
  onMove: (from: number, to: number) => void
}

function Card({ kind, index, count, label, title, meta, visual, tile, onRemove, onMove }: CardProps) {
  const [over, setOver] = useState(false)
  const accepts = (e: DragEvent) => e.dataTransfer.types.includes(DRAG_TYPE)

  const onDrop = (e: DragEvent) => {
    if (!accepts(e)) return
    e.preventDefault()
    e.stopPropagation()
    setOver(false)
    const [fromKind, from] = e.dataTransfer.getData(DRAG_TYPE).split(':')
    if (fromKind === kind && from !== undefined) onMove(Number(from), index)
  }

  // Klavyeyle sıralama: Alt+Sol/Sağ ok; Delete ile kaldırma.
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault()
      const to = index + (e.key === 'ArrowLeft' ? -1 : 1)
      if (to >= 0 && to < count) onMove(index, to)
    } else if (e.key === 'Delete') {
      e.preventDefault()
      onRemove()
    }
  }

  return (
    <div
      className={`attachment g2-card pop-in${tile ? ' g2-card-image' : ''}${over ? ' g2-card-over' : ''}`}
      title={title}
      draggable
      tabIndex={0}
      role="listitem"
      aria-label={`${label}, ${meta}. Sıra ${index + 1}/${count}. Alt ve ok tuşlarıyla taşı, Delete ile kaldır.`}
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_TYPE, `${kind}:${index}`)
        e.dataTransfer.effectAllowed = 'move'
      }}
      onDragOver={(e) => {
        if (!accepts(e)) return
        e.preventDefault()
        e.stopPropagation()
        setOver(true)
      }}
      onDragLeave={() => setOver(false)}
      onDrop={onDrop}
      onKeyDown={onKeyDown}
    >
      {visual}
      {!tile && <span className="attachment-name">{label}</span>}
      {!tile && <span className="g2-card-meta">{meta}</span>}
      <button type="button" className="attachment-remove" onClick={onRemove} aria-label={`${label} ekini kaldır`}>
        <X size={12} aria-hidden="true" />
      </button>
    </div>
  )
}

interface AttachmentCardsProps {
  files: readonly string[]
  images: readonly PendingImage[]
  stats: readonly G2FileStat[] | undefined
  onRemoveFile: (path: string) => void
  onRemoveImage: (id: string) => void
  onMoveFile: (from: number, to: number) => void
  onMoveImage: (from: number, to: number) => void
}

/** Ek önizleme kartları: görseller küçük resim, dosyalar simge + boyut; sürükleyerek sıralanır. */
export function AttachmentCards(props: AttachmentCardsProps) {
  const statOf = (p: string) => props.stats?.find((s) => s.path === p)
  return (
    <div className="attachments g2-cards" role="list" aria-label="Ekler">
      {props.files.map((p, i) => {
        const stat = statOf(p)
        const isDir = stat?.isDir === true
        return (
          <Card
            key={p}
            kind="file"
            index={i}
            count={props.files.length}
            label={p.split('/').pop() || p}
            title={p}
            meta={isDir ? 'klasör ağacı' : stat?.size != null ? formatBytes(stat.size) : 'dosya'}
            visual={isDir ? <Folder size={18} aria-hidden="true" /> : <FileText size={18} aria-hidden="true" />}
            onRemove={() => props.onRemoveFile(p)}
            onMove={props.onMoveFile}
          />
        )
      })}
      {props.images.map((img, i) => (
        <Card
          key={img.id}
          kind="image"
          index={i}
          count={props.images.length}
          label={img.name}
          title={img.name}
          meta={formatBytes(img.size)}
          tile
          visual={<img className="g2-thumb" src={img.dataUrl} alt="" draggable={false} />}
          onRemove={() => props.onRemoveImage(img.id)}
          onMove={props.onMoveImage}
        />
      ))}
    </div>
  )
}
