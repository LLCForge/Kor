// [g3] Akışın sağında kayan içindekiler: bölümler çizgi olarak durur, üzerine gelince başlıklar açılır.
import { useEffect, useState, type RefObject } from 'react'
import { chapterAnchor, type Chapter } from './g3-state.ts'

/** Okuma çizgisi: görünümün üstünden bu kadar aşağıdaki başlık "etkin" sayılır. */
const READ_LINE_PX = 96

/** `onMissing`: başlık DOM'da değilse (uzun sohbette pencere dışı, ADR 0062) önce tamamını açtırır. */
export function ChapterToc({ chapters, scrollRef, onMissing }: { chapters: readonly Chapter[]; scrollRef: RefObject<HTMLElement | null>; onMissing?: () => void }) {
  const [active, setActive] = useState<string | null>(null)

  useEffect(() => {
    const scroller = scrollRef.current
    if (!scroller || chapters.length === 0) return
    let frame = 0
    const update = () => {
      frame = 0
      const box = scroller.getBoundingClientRect()
      // En alttayken okuma çizgisine hiç ulaşamayan son bölümler de seçilebilsin.
      const atEnd = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 4
      const line = atEnd ? box.bottom - READ_LINE_PX : box.top + READ_LINE_PX
      let current = chapters[0]?.id ?? null
      for (const c of chapters) {
        const el = document.getElementById(chapterAnchor(c.id))
        if (el && el.getBoundingClientRect().top <= line) current = c.id
      }
      setActive(current)
    }
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(update)
    }
    update()
    scroller.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      scroller.removeEventListener('scroll', onScroll)
      cancelAnimationFrame(frame)
    }
  }, [chapters, scrollRef])

  if (chapters.length === 0) return null

  return (
    <nav className="g3-toc" aria-label="Bölümler">
      <ol>
        {chapters.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              className={`g3-toc-item${active === c.id ? ' active' : ''}`}
              aria-current={active === c.id ? 'location' : undefined}
              title={c.summary ?? c.title}
              onClick={() => {
                const jump = () => document.getElementById(chapterAnchor(c.id))?.scrollIntoView({ behavior: 'smooth', block: 'start' })
                if (document.getElementById(chapterAnchor(c.id)) || !onMissing) return jump()
                onMissing()
                requestAnimationFrame(() => requestAnimationFrame(jump))
              }}
            >
              <span className="g3-toc-bar" aria-hidden="true" />
              <span className="g3-toc-label">{c.title}</span>
            </button>
          </li>
        ))}
      </ol>
    </nav>
  )
}
