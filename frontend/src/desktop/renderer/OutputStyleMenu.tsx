// [g3] Çıktı stili seçici (Varsayılan / Açıklayıcı / Kısa / Öğretici); seçim ana süreçte kalıcı tutulur.
import { Check, Feather } from 'lucide-react'
import useSWR from 'swr'
import { OUTPUT_STYLES, type OutputStyle } from '../../core/output-styles.ts'
import { api, errorMessage } from './api.ts'
import { usePopover } from './use-popover.ts'

export function OutputStyleMenu() {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const { data: style, error, mutate } = useSWR('g3-output-style', () => api.g3GetOutputStyle(), { revalidateOnFocus: false })
  const current = OUTPUT_STYLES.find((s) => s.id === style) ?? OUTPUT_STYLES[0]

  const choose = async (id: OutputStyle) => {
    setOpen(false)
    try {
      await mutate(
        async () => {
          await api.g3SetOutputStyle(id)
          return id
        },
        { optimisticData: id, rollbackOnError: true },
      )
    } catch (err) {
      console.error('Çıktı stili kaydedilemedi:', errorMessage(err))
    }
  }

  return (
    <div className="popover-anchor" ref={ref}>
      <button
        type="button"
        className={`action-btn g3-style-trigger${style && style !== 'default' ? ' active' : ''}`}
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Çıktı stili: ${current?.label ?? 'Varsayılan'}`}
        title={error ? errorMessage(error) : `Çıktı stili: ${current?.label ?? 'Varsayılan'}`}
      >
        <Feather size={15} aria-hidden="true" />
        {style && style !== 'default' && <span className="g3-style-name">{current?.label}</span>}
      </button>
      {open && (
        <div className="menu pop-in g3-style-menu" role="menu" aria-label="Çıktı stili">
          <p className="menu-heading">Çıktı stili</p>
          {OUTPUT_STYLES.map((s) => (
            <button
              key={s.id}
              type="button"
              role="menuitemradio"
              aria-checked={s.id === current?.id}
              className="mode-item"
              onClick={() => void choose(s.id)}
            >
              <span className="mode-main">
                <span className="mode-label">{s.label}</span>
                <span className="mode-desc">{s.desc}</span>
              </span>
              <span className="mode-check">{s.id === current?.id && <Check size={16} aria-hidden="true" />}</span>
            </button>
          ))}
          <p className="menu-note">Yeni stil açık sohbetlerin sistem istemine hemen uygulanır.</p>
        </div>
      )}
    </div>
  )
}
