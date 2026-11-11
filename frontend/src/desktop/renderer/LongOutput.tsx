// [g3] Uzun araç/komut çıktısı: ilk satırlar gösterilir, kalanı sayfa sayfa ("Daha fazla") veya tamamen açılır.
import { useMemo, useState } from 'react'

const PAGE_LINES = 200
const FIRST_PAGE_LINES = 120

export function LongOutput({ text, className = 'tool-output' }: { text: string; className?: string }) {
  const lines = useMemo(() => text.split('\n'), [text])
  const [shown, setShown] = useState(FIRST_PAGE_LINES)
  const visible = Math.min(shown, lines.length)
  const hidden = lines.length - visible

  return (
    <div className="g3-long-output">
      <pre className={className}>{hidden > 0 ? lines.slice(0, visible).join('\n') : text}</pre>
      {hidden > 0 && (
        <div className="g3-more">
          <span className="g3-more-count">{hidden} satır daha</span>
          <button type="button" className="btn small" onClick={() => setShown((n) => n + PAGE_LINES)}>
            Daha fazla göster
          </button>
          <button type="button" className="btn small ghost" onClick={() => setShown(lines.length)}>
            Tümünü göster
          </button>
        </div>
      )}
    </div>
  )
}
