import type { StoryDefault } from '@ladle/react'
import { useState } from 'react'
import { Select } from '../Select.tsx'

export default { title: 'kor-ui / Select' } satisfies StoryDefault

const LANGS = [
  { value: 'tr', label: 'Türkçe' },
  { value: 'en', label: 'English' },
  { value: 'fr', label: 'Français' },
] as const

type Lang = (typeof LANGS)[number]['value']

export const Default = () => {
  const [v, setV] = useState<Lang>('tr')
  return <Select label="Arayüz dili" value={v} options={LANGS} onChange={setV} />
}

export const WithHint = () => {
  const [v, setV] = useState<Lang>('en')
  return <Select label="Arayüz dili" value={v} options={LANGS} onChange={setV} hint="CLI ile ortak: ui.language" />
}

export const HiddenLabel = () => {
  const [v, setV] = useState<Lang>('fr')
  return <Select label="Arayüz dili" hideLabel value={v} options={LANGS} onChange={setV} />
}

export const DisabledOption = () => {
  const [v, setV] = useState<'manual' | 'auto' | 'plan'>('manual')
  return (
    <Select
      label="Mod"
      value={v}
      onChange={setV}
      options={[
        { value: 'manual', label: 'Manuel' },
        { value: 'auto', label: 'Otomatik onay', disabled: true },
        { value: 'plan', label: 'Plan' },
      ]}
    />
  )
}

export const Disabled = () => <Select label="Arayüz dili" value="tr" options={LANGS} onChange={() => undefined} disabled />
