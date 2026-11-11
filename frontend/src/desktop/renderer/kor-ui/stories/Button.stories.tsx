import type { StoryDefault } from '@ladle/react'
import { ArrowUp, Plus, Trash2 } from 'lucide-react'
import { Button } from '../Button.tsx'

export default { title: 'kor-ui / Button' } satisfies StoryDefault

export const Variants = () => (
  <div className="kor-story-row">
    <Button variant="primary">Gönder</Button>
    <Button>İptal</Button>
    <Button variant="ghost">Atla</Button>
    <Button variant="danger" icon={<Trash2 size={16} aria-hidden="true" />}>
      Sil
    </Button>
  </div>
)

export const Sizes = () => (
  <div className="kor-story-row">
    <Button size="small">Küçük</Button>
    <Button>Varsayılan</Button>
  </div>
)

export const States = () => (
  <div className="kor-story-row">
    <Button variant="primary" disabled>
      Devre dışı
    </Button>
    <Button variant="primary" loading>
      Gönderiliyor
    </Button>
    <Button autoFocus>Odakta</Button>
  </div>
)

export const IconOnly = () => (
  <div className="kor-story-row">
    <Button icon={<Plus size={16} aria-hidden="true" />} aria-label="Dosya ekle" title="Dosya ekle" />
    <Button variant="primary" icon={<ArrowUp size={16} aria-hidden="true" />} aria-label="Gönder" title="Gönder" />
  </div>
)
