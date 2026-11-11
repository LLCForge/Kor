import type { StoryDefault } from '@ladle/react'
import { Mic, Paperclip } from 'lucide-react'
import { Tooltip } from '../Tooltip.tsx'

export default { title: 'kor-ui / Tooltip' } satisfies StoryDefault

export const HoverOrFocus = () => (
  <div className="kor-story-row kor-story-pad">
    <Tooltip text="Dosya ekle (@)">
      <button type="button" className="action-btn" aria-label="Dosya ekle">
        <Paperclip size={16} aria-hidden="true" />
      </button>
    </Tooltip>
    <Tooltip text="Sesle yaz" side="bottom">
      <button type="button" className="action-btn" aria-label="Sesle yaz">
        <Mic size={16} aria-hidden="true" />
      </button>
    </Tooltip>
  </div>
)

export const OpenTop = () => (
  <div className="kor-story-pad">
    <Tooltip text="Dosya ekle (@)" defaultOpen>
      <button type="button" className="action-btn" aria-label="Dosya ekle">
        <Paperclip size={16} aria-hidden="true" />
      </button>
    </Tooltip>
  </div>
)

export const OpenBottom = () => (
  <div className="kor-story-pad">
    <Tooltip text="Sesle yaz" side="bottom" defaultOpen>
      <button type="button" className="action-btn" aria-label="Sesle yaz">
        <Mic size={16} aria-hidden="true" />
      </button>
    </Tooltip>
  </div>
)
