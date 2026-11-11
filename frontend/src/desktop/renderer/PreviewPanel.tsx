// cdesktop "Preview" (Dev Server Preview + Browser Companion): dev sunucusunu başlat/durdur, URL çubuğu, geri /
// yenile / incele (öğe seç) / cihaz genişliği / dışarıda aç; sayfa ana süreçteki WebContentsView'de çizilir ve bu
// bileşen ona yer ayırıp sınırlarını (ResizeObserver) bildirir. Açık <dialog> varken görünüm gizlenir (üstte kalırdı).
import { ArrowLeft, ExternalLink, Monitor, Play, RotateCw, Settings2, Smartphone, Square, SquareDashedMousePointer, Tablet, Terminal, Wrench } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { DevServerStatus, PreviewEvent } from '../ipc-preview.ts'
import { api, errorMessage } from './api.ts'
import { openG7Dialog } from './g7-bus.ts'
import { addPreviewSelection } from './preview-store.ts'

type Device = 'desktop' | 'tablet' | 'mobile'
const DEVICE_WIDTH: Record<Device, number | null> = { desktop: null, tablet: 768, mobile: 390 }
const EMPTY: DevServerStatus = { running: false, command: null, url: null, pid: null, exitCode: null, log: [] }

export function PreviewPanel() {
  const host = useRef<HTMLDivElement>(null)
  const [status, setStatus] = useState<DevServerStatus>(EMPTY)
  const [nav, setNav] = useState<{ url: string; title: string; canGoBack: boolean; loading: boolean }>({ url: '', title: '', canGoBack: false, loading: false })
  const [address, setAddress] = useState<string | null>(null)
  const [inspect, setInspect] = useState(false)
  const [device, setDevice] = useState<Device>('desktop')
  const [logOpen, setLogOpen] = useState(false)
  const [consoleLines, setConsoleLines] = useState<Array<{ level: string; text: string }>>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const shown = useRef(false)

  // Görünümün yeri: host kutusunun pencere içindeki dikdörtgeni (cihaz genişliği seçiliyse ortalanmış dar sütun).
  const bounds = () => {
    const el = host.current
    if (!el) return null
    const r = el.getBoundingClientRect()
    const w = DEVICE_WIDTH[device]
    const width = w ? Math.min(w, r.width) : r.width
    return { x: r.left + (r.width - width) / 2, y: r.top, width, height: r.height }
  }

  useEffect(() => {
    void api.preview.devStatus().then(setStatus).catch((err: unknown) => setError(errorMessage(err)))
    return api.preview.onEvent((ev: PreviewEvent) => {
      if (ev.type === 'dev') setStatus(ev.status)
      else if (ev.type === 'nav') setNav({ url: ev.url, title: ev.title, canGoBack: ev.canGoBack, loading: ev.loading })
      else if (ev.type === 'pick') addPreviewSelection(ev.selection)
      else if (ev.type === 'console') setConsoleLines((l) => [...l.slice(-199), { level: ev.level, text: ev.text }])
      else if (ev.type === 'loadError') setError(`${ev.url}: ${ev.message}`)
      else if (ev.type === 'open' && ev.url) setNav((n) => ({ ...n, url: ev.url! }))
    })
  }, [])

  const target = nav.url || status.url
  // Görünümü göster/konumla; kaldırılınca gizle.
  useLayoutEffect(() => {
    const el = host.current
    if (!el || !target) return
    const b = bounds()
    if (!b) return
    shown.current = true
    void api.preview.show(target, b).catch((err: unknown) => setError(errorMessage(err)))
    const ro = new ResizeObserver(() => { const nb = bounds(); if (nb) void api.preview.setBounds(nb).catch(() => undefined) })
    ro.observe(el)
    const onScroll = () => { const nb = bounds(); if (nb) void api.preview.setBounds(nb).catch(() => undefined) }
    window.addEventListener('resize', onScroll)
    // Açık diyalog varken görünüm üstte kalır → gizle.
    const mo = new MutationObserver(() => {
      const dialog = document.querySelector('dialog[open]')
      if (dialog) void api.preview.hide().catch(() => undefined)
      else { const nb = bounds(); if (nb) void api.preview.show(null, nb).catch(() => undefined) }
    })
    mo.observe(document.body, { childList: true, subtree: true, attributeFilter: ['open'] })
    return () => {
      ro.disconnect()
      mo.disconnect()
      window.removeEventListener('resize', onScroll)
      shown.current = false
      void api.preview.hide().catch(() => undefined)
    }
  }, [target, device]) // eslint-disable-line react-hooks/exhaustive-deps

  const start = () => {
    setBusy(true)
    setError(null)
    void api.preview.devStart().then(setStatus).catch((err: unknown) => setError(errorMessage(err))).finally(() => setBusy(false))
  }
  const stop = () => void api.preview.devStop().then(setStatus).catch((err: unknown) => setError(errorMessage(err)))
  const go = (raw: string) => {
    setAddress(null)
    void api.preview.navigate(raw).catch((err: unknown) => setError(errorMessage(err)))
  }
  const toggleInspect = () => {
    const next = !inspect
    setInspect(next)
    void api.preview.setInspect(next).catch(() => undefined)
  }

  const logTail = status.log.slice(-40)
  const errorLines = consoleLines.filter((l) => l.level === 'error')
  const errors = errorLines.length
  // [cdx] Konsol hatasını tek tıkla ajana gönder: taslağa yazılır, kullanıcı Enter'a basar.
  const sendToAgent = (lines: ReadonlyArray<{ text: string }>) => {
    const body = lines.map((l) => l.text).join('\n').slice(0, 6000)
    window.dispatchEvent(new CustomEvent('cdx:draft', { detail: `Web önizleme konsolunda hata var (${target ?? ''}):\n\`\`\`\n${body}\n\`\`\`\nKaynağını bul ve düzelt; sonra önizlemeyi yenileyip hatanın gittiğini doğrula.` }))
    document.querySelector<HTMLTextAreaElement>('.composer textarea')?.focus()
  }

  return (
    <div className="cdx-preview">
      <div className="cdx-preview-bar">
        <button type="button" className="action-btn" onClick={() => void api.preview.back()} disabled={!nav.canGoBack} aria-label="Geri" title="Geri"><ArrowLeft size={14} aria-hidden="true" /></button>
        <button type="button" className="action-btn" onClick={() => void api.preview.reload()} disabled={!target} aria-label="Yenile" title="Yenile"><RotateCw size={14} aria-hidden="true" className={nav.loading ? 'cdx-spin' : undefined} /></button>
        <input
          className="text-input cdx-preview-url"
          value={address ?? target ?? ''}
          onChange={(e) => setAddress(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') go((e.target as HTMLInputElement).value); if (e.key === 'Escape') setAddress(null) }}
          onBlur={() => setAddress(null)}
          placeholder="http://localhost:3000"
          spellCheck={false}
          aria-label="Önizleme adresi"
        />
        <button type="button" className={`action-btn${inspect ? ' active' : ''}`} onClick={toggleInspect} disabled={!target} aria-pressed={inspect} aria-label="Öğe seç" title="Öğe seç: sayfada tıkladığın öğe mesaja eklenir (Esc kapatır)"><SquareDashedMousePointer size={14} aria-hidden="true" /></button>
        <span className="cdx-preview-devices" role="group" aria-label="Cihaz genişliği">
          {([['desktop', Monitor, 'Masaüstü'], ['tablet', Tablet, 'Tablet (768)'], ['mobile', Smartphone, 'Mobil (390)']] as const).map(([id, Icon, label]) => (
            <button key={id} type="button" className={`action-btn${device === id ? ' active' : ''}`} onClick={() => setDevice(id)} aria-pressed={device === id} aria-label={label} title={label}><Icon size={14} aria-hidden="true" /></button>
          ))}
        </span>
        <button type="button" className="action-btn" onClick={() => void api.preview.openExternal()} disabled={!target} aria-label="Tarayıcıda aç" title="Tarayıcıda aç"><ExternalLink size={14} aria-hidden="true" /></button>
        {status.running ? (
          <button type="button" className="btn small cdx-btn-danger" onClick={stop} title={`Dev sunucusunu durdur (pid ${status.pid ?? '?'})`}><Square size={12} aria-hidden="true" /> Durdur</button>
        ) : (
          <button type="button" className="btn small" onClick={start} disabled={busy || !status.command} title={status.command ?? 'Dev betiği tanımlı değil'}><Play size={12} aria-hidden="true" /> Başlat</button>
        )}
      </div>
      {error && <p className="error-text cdx-preview-error" role="alert">{error}</p>}
      {inspect && <p className="cdx-preview-hint" role="status"><SquareDashedMousePointer size={12} aria-hidden="true" /> Sayfada bir öğeye tıkla; seçim mesaj kutusuna eklenir. Esc ile çık.</p>}
      <div ref={host} className={`cdx-preview-host device-${device}`}>
        {!target && (
          <div className="cdx-preview-empty">
            <Monitor size={28} aria-hidden="true" />
            <h3>{status.running ? 'Sunucu başlıyor…' : 'Dev sunucusu çalışmıyor'}</h3>
            {status.command ? (
              <p className="muted">Betik: <code className="g7-mono">{status.command}</code>{status.exitCode != null && !status.running ? ` · son çıkış ${status.exitCode}` : ''}</p>
            ) : (
              <p className="muted">Ayarlar &gt; Genel &gt; Betikler'de bir dev sunucu komutu tanımla (ör. <code className="g7-mono">npm run dev</code>).</p>
            )}
            <div className="cdx-preview-empty-actions">
              {status.running ? null : <button type="button" className="btn primary" onClick={start} disabled={busy || !status.command}><Play size={14} aria-hidden="true" /> Başlat</button>}
              <button type="button" className="btn" onClick={() => openG7Dialog({ kind: 'settings', section: 'general' })}><Settings2 size={14} aria-hidden="true" /> Betiği düzenle</button>
              <button type="button" className="btn" onClick={() => go('http://localhost:3000')}>localhost:3000'i aç</button>
            </div>
          </div>
        )}
      </div>
      <div className="cdx-preview-foot">
        <button type="button" className="cdx-preview-foot-toggle" onClick={() => setLogOpen((o) => !o)} aria-expanded={logOpen}>
          <Terminal size={12} aria-hidden="true" />
          {status.running ? `Sunucu çalışıyor${status.pid ? ` · pid ${status.pid}` : ''}` : 'Sunucu günlüğü'}
          {logTail.length > 0 && <span className="count">{status.log.length}</span>}
          {errors > 0 && <span className="cdx-preview-errors">{errors} konsol hatası</span>}
        </button>
        {errors > 0 && (
          <button type="button" className="btn small cdx-preview-fix" onClick={() => sendToAgent(errorLines.slice(-5))} title="Son konsol hatalarını mesaj kutusuna yazar">
            <Wrench size={12} aria-hidden="true" /> Hataları ajana gönder
          </button>
        )}
        {logOpen && (
          <pre className="cdx-preview-log">
            {logTail.length === 0 && consoleLines.length === 0 ? 'Henüz çıktı yok.' : null}
            {logTail.map((l, i) => <span key={`l${i}`}>{l}{'\n'}</span>)}
            {consoleLines.slice(-20).map((l, i) => (
              <span key={`c${i}`} className={`cdx-console-${l.level}`}>
                [{l.level}] {l.text}
                {l.level === 'error' && <button type="button" className="cdx-console-fix" onClick={() => sendToAgent([l])} title="Bu hatayı ajana gönder">düzelt</button>}
                {'\n'}
              </span>
            ))}
          </pre>
        )}
      </div>
    </div>
  )
}
