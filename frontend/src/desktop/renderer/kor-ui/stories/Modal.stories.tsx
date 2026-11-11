import type { StoryDefault } from '@ladle/react'
import { useState } from 'react'
import { Button } from '../Button.tsx'
import { Modal } from '../Modal.tsx'

export default { title: 'kor-ui / Modal' } satisfies StoryDefault

export const Confirm = () => {
  const [open, setOpen] = useState(true)
  return (
    <>
      <Button onClick={() => setOpen(true)}>Aç</Button>
      {open && (
        <Modal
          title="Çalışan görev var"
          description="Pencere kapatılırsa ajan durdurulur."
          onClose={() => setOpen(false)}
          footer={
            <>
              <Button onClick={() => setOpen(false)}>Vazgeç</Button>
              <Button variant="danger" onClick={() => setOpen(false)}>
                Kapat
              </Button>
            </>
          }
        />
      )}
    </>
  )
}

export const WithBody = () => {
  const [open, setOpen] = useState(true)
  return (
    <>
      <Button onClick={() => setOpen(true)}>Aç</Button>
      {open && (
        <Modal title="Sürüm notları" onClose={() => setOpen(false)} footer={<Button variant="primary" onClick={() => setOpen(false)}>Tamam</Button>}>
          <ul>
            <li>Yeni tema: Fraunces, Inter, JetBrains Mono</li>
            <li>Kor bağlam menüsü</li>
          </ul>
        </Modal>
      )}
    </>
  )
}
