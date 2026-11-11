// Temaya uygun özel başlık çubuğu. Yerel pencere çerçevesi kapalıdır (tauri.conf.json: decorations=false);
// sürükleme `data-tauri-drag-region` ile, küçült/büyüt/kapat bu çubuktaki temalı düğmelerle yapılır.
import { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ShieldCheck, WifiOff } from 'lucide-react'
import useSWR from 'swr'
import { api } from './api.ts'
import { Logo } from './Logo.tsx'
import { TitleItems } from './shellui/TitleItems.tsx'
import { Tooltip } from './kor-ui/index.ts'

function WindowControls() {
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    const win = getCurrentWindow()
    let stop: (() => void) | null = null
    let alive = true
    void win.isMaximized().then((m) => alive && setMaximized(m))
    void win
      .onResized(() => void win.isMaximized().then((m) => alive && setMaximized(m)))
      .then((un) => {
        if (alive) stop = un
        else un()
      })
    return () => {
      alive = false
      stop?.()
    }
  }, [])

  const win = getCurrentWindow()
  return (
    <div className="titlebar-controls">
      <Tooltip side="bottom" text="Küçült">
        <button type="button" className="titlebar-btn" aria-label="Küçült" onClick={() => void win.minimize()}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
          </svg>
        </button>
      </Tooltip>
      <Tooltip side="bottom" text={maximized ? 'Önceki boyut' : 'Büyüt'}>
        <button
          type="button"
          className="titlebar-btn"
          aria-label={maximized ? 'Önceki boyut' : 'Büyüt'}
          onClick={() => void win.toggleMaximize()}
        >
          {maximized ? (
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
              <path d="M2.5 2.5V0.5h7v7h-2M0.5 2.5h7v7h-7z" fill="none" stroke="currentColor" strokeWidth="1" />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
              <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" />
            </svg>
          )}
        </button>
      </Tooltip>
      <Tooltip side="bottom" text="Kapat">
        <button type="button" className="titlebar-btn titlebar-close" aria-label="Kapat" onClick={() => void win.close()}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" stroke="currentColor" strokeWidth="1" />
          </svg>
        </button>
      </Tooltip>
    </div>
  )
}

// §6.5: gizlilik/çevrimdışı mod açıkken her ekranda görünür kalır (ADR 0035). Anahtar CoreSection ile ortak:
// orada açılıp kapatılınca rozet hemen değişir.
function NetworkBadge() {
  const { data } = useSWR('core:privacy', () => api.core.privacy())
  if (!data?.enabled && !data?.offline) return null
  const offline = data.offline === true
  const tip = offline
    ? 'Çevrimdışı mod: Kor bu bilgisayar dışında hiçbir yere bağlanmaz.'
    : 'Gizlilik modu: yalnızca yerel sağlayıcılar; Kor dışarıya veri göndermez.'
  return (
    <span className="titlebar-badge" role="status" title={tip} aria-label={tip} data-kor-cmd="core.privacy">
      {offline ? <WifiOff size={14} aria-hidden="true" /> : <ShieldCheck size={14} aria-hidden="true" />}
      {offline ? 'Çevrimdışı' : 'Gizlilik'}
    </span>
  )
}

export function TitleBar() {
  return (
    <div className="titlebar" data-tauri-drag-region>
      <Logo size={18} />
      <span className="titlebar-name" data-tauri-drag-region>
        Kor
      </span>
      <NetworkBadge />
      {/* ADR 0051: Güncelle pill'i, onay kuyruğu, zil (sağda, pencere düğmelerinin yanında) */}
      <TitleItems />
      <WindowControls />
    </div>
  )
}
