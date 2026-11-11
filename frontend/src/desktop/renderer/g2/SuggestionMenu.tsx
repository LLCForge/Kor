import { useEffect, useRef, type ReactNode } from 'react'

export interface SuggestionItem {
  key: string
  label: string
  description?: string
  icon?: ReactNode
}

interface SuggestionMenuProps {
  /** Liste kimliği; textarea aria-controls/aria-activedescendant ile bağlanır */
  id: string
  title: string
  items: readonly SuggestionItem[]
  activeIndex: number
  loading: boolean
  emptyText: string
  onSelect: (index: number) => void
  onHover: (index: number) => void
}

/** "/" komutları ve "@" dosya önerileri için açılır liste; klavye yönetimi textarea'dadır (odak orada kalır). */
export function SuggestionMenu({ id, title, items, activeIndex, loading, emptyText, onSelect, onHover }: SuggestionMenuProps) {
  const listRef = useRef<HTMLUListElement>(null)

  useEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex, items])

  return (
    <div className="g2-popup pop-in">
      <p className="menu-heading">{title}</p>
      {items.length === 0 ? (
        <p className="menu-note" role="status">{loading ? 'Aranıyor...' : emptyText}</p>
      ) : (
        <ul ref={listRef} id={id} role="listbox" aria-label={title} className="menu-list">
          {items.map((item, i) => (
            <li
              key={item.key}
              id={`${id}-${i}`}
              role="option"
              aria-selected={i === activeIndex}
              className={`g2-option${i === activeIndex ? ' active' : ''}`}
              // Tıklama textarea'dan odağı almasın.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onSelect(i)}
              onMouseEnter={() => onHover(i)}
            >
              {item.icon && <span className="g2-option-icon" aria-hidden="true">{item.icon}</span>}
              <span className="menu-text">
                <span className="g2-option-label">{item.label}</span>
                {item.description && <span className="menu-desc">{item.description}</span>}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
