import type { StoryDefault } from '@ladle/react'
import { RefreshCw } from 'lucide-react'
import { Button } from '../Button.tsx'
import { Card } from '../Card.tsx'

export default { title: 'kor-ui / Card' } satisfies StoryDefault

export const Surface = () => (
  <Card title="Maliyet">
    <p>Bu oturum ≈ $0.004</p>
  </Card>
)

export const Raised = () => (
  <Card title="Sistem sağlığı" layer="raised">
    <p>Disk %71 dolu</p>
  </Card>
)

export const WithActions = () => (
  <Card title="Kota" actions={<Button size="small" icon={<RefreshCw size={14} aria-hidden="true" />} aria-label="Yenile" title="Yenile" />}>
    <p>Günlük kota: 42 / 100 istek</p>
  </Card>
)

export const Untitled = () => (
  <Card>
    <p>Başlıksız kart içerik taşır.</p>
  </Card>
)
