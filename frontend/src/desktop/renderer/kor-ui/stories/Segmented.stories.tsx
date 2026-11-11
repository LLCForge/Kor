import type { StoryDefault } from '@ladle/react'
import { Eye, PenLine } from 'lucide-react'
import { useState } from 'react'
import { Segmented } from '../Segmented.tsx'

export default { title: 'kor-ui / Segmented' } satisfies StoryDefault

export const WritePreview = () => {
  const [v, setV] = useState<'write' | 'preview'>('write')
  return (
    <Segmented
      label="Taslak görünümü"
      value={v}
      onChange={setV}
      options={[
        { value: 'write', label: 'Yaz', icon: <PenLine size={12} aria-hidden="true" /> },
        { value: 'preview', label: 'Önizle', icon: <Eye size={12} aria-hidden="true" /> },
      ]}
    />
  )
}

export const ThreeWay = () => {
  const [v, setV] = useState<'manual' | 'auto' | 'plan'>('plan')
  return (
    <Segmented
      label="Mod"
      value={v}
      onChange={setV}
      options={[
        { value: 'manual', label: 'Manuel' },
        { value: 'auto', label: 'Otomatik onay' },
        { value: 'plan', label: 'Plan' },
      ]}
    />
  )
}

export const WithDisabled = () => {
  const [v, setV] = useState<'a' | 'b' | 'c'>('a')
  return (
    <Segmented
      label="Örnek"
      value={v}
      onChange={setV}
      options={[
        { value: 'a', label: 'Bir' },
        { value: 'b', label: 'İki', disabled: true },
        { value: 'c', label: 'Üç' },
      ]}
    />
  )
}
