import type { StoryDefault } from '@ladle/react'
import { Button } from '../Button.tsx'
import { Toaster, toast } from '../Toast.tsx'

export default { title: 'kor-ui / Toast' } satisfies StoryDefault

export const AllKinds = () => (
  <div className="kor-story-row">
    <Button onClick={() => toast('Oturum kaydedildi', 'success')}>Başarı</Button>
    <Button onClick={() => toast('Yeni sürüm hazır: 0.9.0')}>Bilgi</Button>
    <Button onClick={() => toast('Bütçenin %80’i kullanıldı', 'warning')}>Uyarı</Button>
    <Button onClick={() => toast('Sağlayıcıya ulaşılamadı: zaman aşımı', 'error')}>Hata</Button>
    <Toaster />
  </div>
)

export const Persistent = () => (
  <div className="kor-story-row">
    <Button onClick={() => toast('Kullanıcı kapatana kadar kalır', 'info', 0)}>Kalıcı bildirim</Button>
    <Toaster />
  </div>
)
