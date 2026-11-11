// Composer "Gelişmiş" menüsü (§23.3, ADR 0053): alt araç çubuğu ≤ 8 kontrol kalsın diye seyrek kullanılan
// seçenekler (akıl yürütme düzeyi, hızlı mod, çıktı stili, ajan profili, Vim tuşları, tam ekran) tek etiketli
// düğmenin arkasındadır. Her seçenek kendi bileşenidir; menüler iç içe açılır (aynı DOM ağacı, portal yok).
import { Keyboard, Maximize2, Minimize2, SlidersHorizontal } from 'lucide-react'
import type { ReactNode } from 'react'
import { Tooltip } from './kor-ui/index.ts'
import { usePopover } from './use-popover.ts'

export interface ComposerMoreProps {
  /** EffortPicker, FastToggle, OutputStyleMenu, ProfileMenu: mevcut bileşenler olduğu gibi */
  effort: ReactNode
  fast: ReactNode
  outputStyle: ReactNode
  profile: ReactNode
  vim: boolean
  onToggleVim: () => void
  fullscreen: boolean
  onToggleFullscreen: () => void
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="kor-composer-more-row">
      <span className="kor-composer-more-label">{label}</span>
      {children}
    </div>
  )
}

export function ComposerMore(p: ComposerMoreProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  return (
    <div className="kor-composer-more" ref={ref}>
      <Tooltip text="Gelişmiş: düzey, hız, stil, profil, Vim, tam ekran">
        <button
          type="button"
          className={`action-btn${open ? ' active' : ''}`}
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label="Gelişmiş composer seçenekleri"
        >
          <SlidersHorizontal size={16} aria-hidden="true" />
        </button>
      </Tooltip>
      {open && (
        <div className="kor-composer-more-panel pop-in" role="dialog" aria-label="Gelişmiş composer seçenekleri">
          <Row label="Akıl yürütme düzeyi">{p.effort}</Row>
          <Row label="Hızlı mod">{p.fast}</Row>
          <Row label="Çıktı stili">{p.outputStyle}</Row>
          <Row label="Ajan profili">{p.profile}</Row>
          <Row label="Tuş düzeni">
            <button
              type="button"
              className={`action-btn g2-tool${p.vim ? ' active' : ''}`}
              onClick={p.onToggleVim}
              aria-pressed={p.vim}
              aria-label="Tuş düzeni: varsayılan, vim, emacs"
              title="Tuş düzeni (varsayılan → vim → emacs). Vim: h j k l w b 0 $ x dd p v y : i a o A I, Esc · Emacs: C-a C-e C-b C-f M-b M-f C-k C-y C-w C-d"
              data-kor-cmd="prefs.setUi"
            >
              <Keyboard size={14} aria-hidden="true" />
            </button>
          </Row>
          <Row label="Tam ekran yaz">
            <button
              type="button"
              className="action-btn g2-tool"
              onClick={p.onToggleFullscreen}
              aria-pressed={p.fullscreen}
              aria-label={p.fullscreen ? 'Tam ekrandan çık' : 'Tam ekran yaz'}
              title={p.fullscreen ? 'Tam ekrandan çık (Esc)' : 'Tam ekran yaz'}
            >
              {p.fullscreen ? <Minimize2 size={14} aria-hidden="true" /> : <Maximize2 size={14} aria-hidden="true" />}
            </button>
          </Row>
        </div>
      )}
    </div>
  )
}
