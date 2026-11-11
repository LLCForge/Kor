// cdesktop/Claude Code tarzı "Tarayıcı" paneli: ajanın izole tarayıcısı. Sekme şeridi, adres çubuğu, geri/ileri/yenile,
// "Ajan: …" son eylem rozeti ve konsol. Sayfa ana süreçte WebContentsView ile çizilir; bu bileşen yer ayırıp sınırlarını
// bildirir (ResizeObserver). Açık <dialog> varken görünüm gizlenir (üstte kalırdı).
import { ArrowLeft, ArrowRight, Bot, Globe, Plus, RotateCw, Terminal, X } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { BrowserPanelEvent, BrowserTabInfo } from '../ipc-browser.ts'
import { api, errorMessage } from './api.ts'

export function BrowserPanel() {
  const host = useRef<HTMLDivElement>(null)
  const [tabs, setTabs] = useState<BrowserTabInfo[]>([])
  const [address, setAddress] = useState<string | null>(null)
  const [action, setAction] = useState<string | null>(null)
  const [logOpen, setLogOpen] = useState(false)
  const [lines, setLines] = useState<Array<{ level: string; text: string }>>([])
  const [error, setError] = useState<string | null>(null)
  const active = tabs.find((t) => t.active) ?? null

  const bounds = () => {
    const el = host.current
    if (!el) return null
    const r = el.getBoundingClientRect()
    return { x: r.left, y: r.top, width: r.width, height: r.height }
  }

  useEffect(() => {
    void api.abrowser.tabs().then(setTabs).catch((err: unknown) => setError(errorMessage(err)))
    void api.abrowser.logs().then((l) => setLines(l.slice(-200))).catch(() => undefined)
    return api.abrowser.onEvent((ev: BrowserPanelEvent) => {
      if (ev.type === 'tabs') setTabs(ev.tabs)
      else if (ev.type === 'console') setLines((l) => [...l.slice(-199), { level: ev.level, text: ev.text }])
      else if (ev.type === 'action') {
        setAction(ev.text)
        setTimeout(() => setAction((a) => (a === ev.text ? null : a)), 6000)
      }
    })
  }, [])

  useLayoutEffect(() => {
    const el = host.current
    if (!el) return
    const b = bounds()
    if (b) void api.abrowser.show(b).catch((err: unknown) => setError(errorMessage(err)))
    const ro = new ResizeObserver(() => { const nb = bounds(); if (nb) void api.abrowser.setBounds(nb).catch(() => undefined) })
    ro.observe(el)
    const onResize = () => { const nb = bounds(); if (nb) void api.abrowser.setBounds(nb).catch(() => undefined) }
    window.addEventListener('resize', onResize)
    const mo = new MutationObserver(() => {
      if (document.querySelector('dialog[open]')) void api.abrowser.hide().catch(() => undefined)
      else { const nb = bounds(); if (nb) void api.abrowser.show(nb).catch(() => undefined) }
    })
    mo.observe(document.body, { childList: true, subtree: true, attributeFilter: ['open'] })
    return () => {
      ro.disconnect()
      mo.disconnect()
      window.removeEventListener('resize', onResize)
      void api.abrowser.hide().catch(() => undefined)
    }
  }, [])

  const go = (raw: string) => {
    setAddress(null)
    const url = /^[a-z]+:\/\//i.test(raw) ? raw : `https://${raw}`
    void api.abrowser.navigate(url).catch((err: unknown) => setError(errorMessage(err)))
  }
  const errors = lines.filter((l) => l.level === 'error').length

  return (
    <div className="cdx-browser">
      <div className="cdx-browser-tabs" role="tablist">
        {tabs.map((t, i) => (
          <div key={t.id} role="tab" aria-selected={t.active} className={`cdx-browser-tab${t.active ? ' on' : ''}`} onClick={() => void api.abrowser.activate(t.id)} title={t.url}>
            <Globe size={11} aria-hidden="true" className={t.loading ? 'cdx-spin' : undefined} />
            <span className="cdx-browser-tab-title">{t.title || t.url || `Sekme ${i + 1}`}</span>
            <button type="button" className="cdx-browser-tab-x" onClick={(e) => { e.stopPropagation(); void api.abrowser.close(t.id) }} aria-label="Sekmeyi kapat"><X size={11} aria-hidden="true" /></button>
          </div>
        ))}
        <button type="button" className="action-btn cdx-browser-newtab" onClick={() => void api.abrowser.newTab()} aria-label="Yeni sekme" title="Yeni sekme"><Plus size={13} aria-hidden="true" /></button>
      </div>
      <div className="cdx-preview-bar">
        <button type="button" className="action-btn" onClick={() => void api.abrowser.back()} disabled={!active?.canGoBack} aria-label="Geri" title="Geri"><ArrowLeft size={14} aria-hidden="true" /></button>
        <button type="button" className="action-btn" onClick={() => void api.abrowser.forward()} disabled={!active} aria-label="İleri" title="İleri"><ArrowRight size={14} aria-hidden="true" /></button>
        <button type="button" className="action-btn" onClick={() => void api.abrowser.reload()} disabled={!active} aria-label="Yenile" title="Yenile"><RotateCw size={14} aria-hidden="true" className={active?.loading ? 'cdx-spin' : undefined} /></button>
        <input
          className="text-input cdx-preview-url"
          value={address ?? active?.url ?? ''}
          onChange={(e) => setAddress(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') go((e.target as HTMLInputElement).value); if (e.key === 'Escape') setAddress(null) }}
          onBlur={() => setAddress(null)}
          placeholder="https://… (Enter ile git)"
          spellCheck={false}
          aria-label="Adres"
        />
        {action && <span className="cdx-browser-action" role="status"><Bot size={12} aria-hidden="true" /> Ajan: {action}</span>}
      </div>
      {error && <p className="error-text cdx-preview-error" role="alert">{error}</p>}
      <div ref={host} className="cdx-preview-host cdx-browser-host">
        {tabs.length === 0 && (
          <div className="cdx-preview-empty">
            <Globe size={28} aria-hidden="true" />
            <h3>Ajan tarayıcısı</h3>
            <p className="muted">Ajan bir adrese gidince sayfa burada görünür; sen de adres çubuğundan gezebilirsin. Çerez ve oturumlar kalıcı değildir; kullanıcının Chrome'una dokunmaz.</p>
            <div className="cdx-preview-empty-actions">
              <button type="button" className="btn" onClick={() => void api.abrowser.newTab('https://duckduckgo.com')}>Yeni sekme aç</button>
            </div>
          </div>
        )}
      </div>
      <div className="cdx-preview-foot">
        <button type="button" className="cdx-preview-foot-toggle" onClick={() => setLogOpen((o) => !o)} aria-expanded={logOpen}>
          <Terminal size={12} aria-hidden="true" />
          Konsol
          {lines.length > 0 && <span className="count">{lines.length}</span>}
          {errors > 0 && <span className="cdx-preview-errors">{errors} hata</span>}
        </button>
        {logOpen && (
          <pre className="cdx-preview-log">
            {lines.length === 0 ? 'Konsol boş.' : lines.slice(-60).map((l, i) => <span key={i} className={`cdx-console-${l.level}`}>[{l.level}] {l.text}{'\n'}</span>)}
          </pre>
        )}
      </div>
    </div>
  )
}
