import type { StoryDefault } from '@ladle/react'
import { Bug, FileText, FolderOpen } from 'lucide-react'
import { useState } from 'react'
import { Chip } from '../Chip.tsx'

export default { title: 'kor-ui / Chip' } satisfies StoryDefault

export const Static = () => <Chip label="src/main.rs" icon={<FileText size={14} aria-hidden="true" />} />

export const Clickable = () => {
  const [on, setOn] = useState(false)
  return <Chip label="Hata bul" icon={<Bug size={14} aria-hidden="true" />} selected={on} onClick={() => setOn((v) => !v)} />
}

export const Selected = () => <Chip label="Plan" selected onClick={() => undefined} />

export const Removable = () => {
  const [shown, setShown] = useState(true)
  return shown ? <Chip label="README.md" icon={<FileText size={14} aria-hidden="true" />} onRemove={() => setShown(false)} /> : <span>Kaldırıldı</span>
}

export const Disabled = () => <Chip label="Klasör aç" icon={<FolderOpen size={14} aria-hidden="true" />} onClick={() => undefined} disabled />

export const LongLabel = () => (
  <div className="kor-story-narrow">
    <Chip label="çok/uzun/bir/klasör/yolu/burada/kesilir/ve/üç/nokta/ile/biter.ts" onRemove={() => undefined} />
  </div>
)
