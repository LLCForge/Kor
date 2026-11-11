// [g3] Akıllı otomatik kaydırma: kullanıcı en alttaysa yeni içerikle birlikte iner, yukarıdaysa yerinde kalır.
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'

/** En alta bu kadar yakınken "altta" sayılır. */
const BOTTOM_SLACK_PX = 80

export function useAutoScroll(
  scrollRef: RefObject<HTMLElement | null>,
  contentRef: RefObject<HTMLElement | null>,
  /** Son öğe yeni bir kullanıcı mesajıysa her durumda en alta inilir */
  userSentKey: string | undefined,
): { atBottom: boolean; unseen: boolean; scrollToBottom: () => void } {
  const atBottomRef = useRef(true)
  const [atBottom, setAtBottom] = useState(true)
  const [unseen, setUnseen] = useState(false)

  const scrollToBottom = useCallback(
    (smooth = false) => {
      const el = scrollRef.current
      if (!el) return
      el.scrollTo({ top: el.scrollHeight, behavior: smooth ? 'smooth' : 'auto' })
      atBottomRef.current = true
      setAtBottom(true)
      setUnseen(false)
    },
    [scrollRef],
  )

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    let lastTop = el.scrollTop
    // Kaydırma olayı bir kare gecikir; bu arada içerik büyümüşse programın kaydırması "yukarıda" sanılır.
    // Bu yüzden yalnızca yukarı doğru kaydırma kullanıcının takibi bırakması sayılır.
    const onScroll = () => {
      const top = el.scrollTop
      const near = el.scrollHeight - top - el.clientHeight < BOTTOM_SLACK_PX
      const movedUp = top < lastTop - 1
      lastTop = top
      if (near) {
        atBottomRef.current = true
        setAtBottom(true)
        setUnseen(false)
      } else if (movedUp) {
        atBottomRef.current = false
        setAtBottom(false)
      }
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    return () => el.removeEventListener('scroll', onScroll)
  }, [scrollRef])

  // İçerik büyüdükçe (akış, açılan kart, çizilen diyagram) yalnızca kullanıcı alttaysa izlenir.
  useEffect(() => {
    const content = contentRef.current
    if (!content) return
    const observer = new ResizeObserver(() => {
      if (atBottomRef.current) scrollToBottom()
      else setUnseen(true)
    })
    observer.observe(content)
    return () => observer.disconnect()
  }, [contentRef, scrollToBottom])

  useLayoutEffect(() => {
    if (userSentKey !== undefined) scrollToBottom()
  }, [userSentKey, scrollToBottom])

  return { atBottom, unseen, scrollToBottom: () => scrollToBottom(true) }
}
