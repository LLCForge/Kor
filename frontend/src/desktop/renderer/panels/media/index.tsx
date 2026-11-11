import { Clapperboard } from 'lucide-react'
import type { KorPanel } from '../registry.ts'
import { MediaPanel } from './MediaPanel.tsx'
import './panel.css'

export const panel: KorPanel = {
  id: 'media',
  label: 'Medya',
  icon: Clapperboard,
  keywords: 'medya görsel resim video ses müzik üret generate image galeri gallery referans upscale arka plan fal venice hugging face katalog tts konuşma',
  component: MediaPanel,
}
