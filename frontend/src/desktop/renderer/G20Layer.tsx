// [g20] Uygulama katmanı: /tuval komutuyla açılan beyaz tahta penceresi.
import { useSyncExternalStore } from 'react'
import { closeBoard, isBoardOpen, subscribeBoard } from './g20-board-store.ts'
import { G7Modal } from './G7Modal.tsx'
import { Whiteboard } from './Whiteboard.tsx'

export function G20Layer() {
  const open = useSyncExternalStore(subscribeBoard, isBoardOpen)
  return (
    <G7Modal open={open} onClose={closeBoard} titleId="g20-board-title" title="Tuval" className="g20-board-modal">
      <Whiteboard />
    </G7Modal>
  )
}
