// [g3] feature 29: Bir yanıtta art arda gelen birden çok diyagram bloğunu sekmeli bir kartta toplar.
//      Tüm paneller monte kalır (yalnızca gizlenir) ki sekme değişince diyagram yeniden çizilmesin ve durum korunsun.
import { Children, useState, type ReactNode } from 'react'

export function DiagramTabs({ labels, children }: { labels: string[]; children: ReactNode }) {
  const panels = Children.toArray(children)
  const [active, setActive] = useState(0)
  const current = Math.min(active, panels.length - 1)

  return (
    <div className="g3-dgm-tabs">
      <div className="g3-dgm-tabbar" role="tablist" aria-label="Diyagramlar">
        {panels.map((_, i) => (
          <button
            key={i}
            type="button"
            role="tab"
            aria-selected={i === current}
            className={`g3-dgm-tab${i === current ? ' g3-on' : ''}`}
            onClick={() => setActive(i)}
          >
            {labels[i] ?? `Diyagram ${i + 1}`}
          </button>
        ))}
      </div>
      {panels.map((panel, i) => (
        <div key={i} role="tabpanel" hidden={i !== current}>
          {panel}
        </div>
      ))}
    </div>
  )
}
