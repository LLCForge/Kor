// §21.3 on the real desktop app (ADR 0053): every browser shortcut §21.1 closes is pressed and nothing of the
// browser happens (no navigation, reload, zoom, fullscreen, new window); the redirected ones reach Kor's own
// action (export / save / search / UI scale). Every right-click context gets Kor's own menu with the spec's
// items (the lists below are the snapshot), never the WebView's default menu.
//
// The default menu and the browser accelerators are suppressed by `preventDefault` in the lock
// (renderer/webview-lock.ts): the test records, after dispatch, that every non-modifier keydown and every
// contextmenu event was default-prevented, and that the page itself did not change.
// Windows: Playwright over WebView2 CDP (a debug build with KOR_DEVTOOLS=1, which §21.1 lets open DevTools,
// so the five devtools chords are checked on the WebDriver run and by the release binary scan instead).
// Linux: tauri-driver (WebDriver), devtools off, every chord checked.
import { test, expect } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { USE_WEBDRIVER, attach } from './app.ts'
import { FRONTEND, SCENARIOS, serve } from './harness.ts'
import { openProject } from './flow.ts'
import type { Driver } from './driver.ts'

interface Case {
  chord: string
  devtools?: true
  /** `kor:shortcut` detail the lock dispatches (Ctrl+P/S/F). */
  redirect?: 'export' | 'save' | 'find'
  /** Kor's UI scale (`html { zoom }`) after the chord. */
  zoom?: string
}

// §21.1 "Kapatılanlar", in order. The zoom chords run as a sequence from scale 1.
const CASES: Case[] = [
  { chord: 'F12', devtools: true },
  { chord: 'Control+Shift+I', devtools: true },
  { chord: 'Control+Shift+J', devtools: true },
  { chord: 'Control+Shift+C', devtools: true },
  { chord: 'Control+U', devtools: true },
  { chord: 'Control+R' },
  { chord: 'F5' },
  { chord: 'Control+Shift+R' },
  { chord: 'Alt+ArrowLeft' },
  { chord: 'Alt+ArrowRight' },
  { chord: 'Backspace' },
  { chord: 'Control+P', redirect: 'export' },
  { chord: 'Control+S', redirect: 'save' },
  { chord: 'Control+F', redirect: 'find' },
  { chord: 'Control+T' },
  { chord: 'Control+N' },
  { chord: 'Control+W' },
  { chord: 'Control+Digit0', zoom: '1' },
  { chord: 'Control+Equal', zoom: '1.1' },
  { chord: 'Control+Minus', zoom: '1' },
  { chord: 'Control+Minus', zoom: '0.9' },
  { chord: 'Control+Digit0', zoom: '1' },
  { chord: 'F11' },
  { chord: 'Control+Shift+Delete' },
]

/** Kor's own context menu per target (Turkish source labels, renderer/webview-lock.ts `menuItems`). */
const MENUS: Array<{ name: string; selector: string; items: string[] }> = [
  { name: 'text field', selector: 'textarea.composer-input', items: ['Kes', 'Kopyala', 'Yapıştır', 'Tümünü seç'] },
  { name: 'code block', selector: '#kl-code', items: ['Kopyala', 'Dosyaya kaydet', "Diff'e uygula"] },
  { name: 'message', selector: '#kl-msg', items: ['Alıntıla', 'Yeniden üret', 'Referans yap'] },
  { name: 'image', selector: '#kl-img', items: ['Kaydet', 'Referans yap', 'Kopyala'] },
  { name: 'plain chrome', selector: '#kl-chrome', items: [] },
]

interface Probe {
  marker: string
  href: string
  history: number
  dpr: number
  outer: string
  zoom: string
  search: boolean
  thread: boolean
  keys: Array<{ key: string; prevented: boolean }>
  menus: boolean[]
  redirects: string[]
  menu: string[] | null
}

type Lock = { marker: string; keys: Probe['keys']; menus: boolean[]; redirects: string[] }

/** Recorders run after the lock: `setTimeout` reads `defaultPrevented` once the dispatch is over. */
async function install(d: Driver): Promise<string> {
  return d.run(() => {
    const w = window as unknown as { __korLock?: Lock }
    const lock: Lock = { marker: Math.random().toString(36).slice(2), keys: [], menus: [], redirects: [] }
    w.__korLock = lock
    window.addEventListener('keydown', (e) => { setTimeout(() => lock.keys.push({ key: e.key, prevented: e.defaultPrevented }), 0) }, true)
    window.addEventListener('contextmenu', (e) => { setTimeout(() => lock.menus.push(e.defaultPrevented), 0) }, true)
    window.addEventListener('kor:shortcut', (e) => { lock.redirects.push(String((e as CustomEvent).detail)) })
    return lock.marker
  })
}

/** The page's state; `null` when the page was replaced (a reload or navigation lost the marker). */
async function probe(d: Driver): Promise<Probe | null> {
  return d.run(() => {
    const lock = (window as unknown as { __korLock?: Lock }).__korLock
    if (!lock) return null
    const menu = document.querySelector('.kor-context-menu')
    return {
      marker: lock.marker,
      href: location.href,
      history: history.length,
      dpr: window.devicePixelRatio,
      outer: `${window.outerWidth}x${window.outerHeight}`,
      zoom: document.documentElement.style.zoom,
      search: document.getElementById('g3-search-input') !== null,
      thread: document.querySelector('.g3-thread') !== null,
      keys: lock.keys.splice(0),
      menus: lock.menus.splice(0),
      redirects: lock.redirects.splice(0),
      menu: menu ? [...menu.querySelectorAll('[role="menuitem"] > span:first-child')].map((s) => s.textContent ?? '') : null,
    }
  })
}

/** Nothing focused, no selection, no open menu or search: the next key goes to the page body. */
async function settle(d: Driver): Promise<void> {
  await d.keys('Escape')
  await d.run(() => {
    ;(document.activeElement as HTMLElement | null)?.blur?.()
    window.getSelection()?.removeAllRanges()
  })
  await new Promise((r) => setTimeout(r, 250))
}

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms))
const MODIFIERS = new Set(['Control', 'Shift', 'Alt', 'Meta'])

/** Turkish source label → the label the app shows in its current language. */
function translator(lang: string): (tr: string) => string {
  if (lang !== 'en' && lang !== 'fr') return (s) => s
  const dict = JSON.parse(readFileSync(join(FRONTEND, `src/desktop/renderer/i18n/${lang}.json`), 'utf8')) as Record<string, string>
  return (s) => dict[s] ?? s
}

test('§21.1 browser shortcuts and right-clicks never reach the WebView', async () => {
  const served = await serve(join(SCENARIOS, 'task-approve-write.yaml'))
  const { d, page } = await attach(served)
  try {
    // No prompt is sent: no session exists, so Ctrl+P/S (export) open no native save dialog.
    await openProject(d, served.project)
    await pause(1500)
    const marker = await install(d)
    const start = await probe(d)
    expect(start).not.toBeNull()
    const pages = page ? page.context().pages().length : 0
    const skipped: string[] = []

    for (const c of CASES) {
      // The CDP attach needs DevTools on (debug build, KOR_DEVTOOLS=1), where §21.1 frees these chords.
      if (c.devtools && !USE_WEBDRIVER) {
        skipped.push(c.chord)
        continue
      }
      await settle(d)
      await probe(d)
      await d.keys(c.chord)
      await pause(400)
      const s = await probe(d)
      expect(s, `${c.chord}: the page reloaded or navigated`).not.toBeNull()
      expect(s!.marker, `${c.chord}: reload`).toBe(marker)
      expect(s!.href, `${c.chord}: navigation`).toBe(start!.href)
      expect(s!.history, `${c.chord}: history entry`).toBe(start!.history)
      expect(s!.dpr, `${c.chord}: browser zoom`).toBe(start!.dpr)
      expect(s!.outer, `${c.chord}: window size (fullscreen)`).toBe(start!.outer)
      const pressed = s!.keys.filter((k) => !MODIFIERS.has(k.key) && k.key !== 'Escape')
      expect(pressed.length, `${c.chord}: keydown reached the page`).toBeGreaterThan(0)
      expect(pressed.filter((k) => !k.prevented), `${c.chord}: default action not prevented`).toEqual([])
      if (c.redirect) expect(s!.redirects, `${c.chord}: Kor's own action`).toContain(c.redirect)
      // The conversation search lives in an open conversation (Thread); the home view has none.
      if (c.redirect === 'find' && s!.thread) expect(s!.search, 'Ctrl+F opens Kor\'s conversation search').toBe(true)
      if (c.zoom) expect(s!.zoom || '1', `${c.chord}: Kor UI scale`).toBe(c.zoom)
      if (page) expect(page.context().pages().length, `${c.chord}: new window or tab`).toBe(pages)
    }
    if (skipped.length) test.info().annotations.push({ type: 'skipped on CDP (DevTools enabled for the attach)', description: skipped.join(', ') })

    // Right-click targets: the composer is real; message, code block, image and a plain chrome element
    // are a fixture with the classes the renderer's messages use (no conversation is needed for it).
    await settle(d)
    await d.run(() => {
      const box = document.createElement('div')
      box.id = 'kl-fixture'
      box.setAttribute('style', 'position:fixed;left:24px;top:96px;z-index:2147483646;width:360px;padding:8px;background:Canvas;color:CanvasText')
      box.innerHTML =
        '<div class="messages"><div class="message assistant"><div class="prose">' +
        '<p id="kl-msg">Merhaba, dosya hazır.</p>' +
        '<pre class="code-block" id="kl-code"><code>let x = 1</code></pre>' +
        '<img id="kl-img" alt="" width="48" height="48" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=">' +
        '</div></div></div><div id="kl-chrome" style="height:32px;user-select:none">Kor</div>'
      document.body.appendChild(box)
    })
    const lang = await d.run(() => document.documentElement.lang)
    const tr = translator(lang)
    const seen: Record<string, string[]> = {}
    for (const m of MENUS) {
      await d.run(() => window.getSelection()?.removeAllRanges())
      await probe(d)
      await d.contextClick(m.selector)
      await pause(400)
      const s = await probe(d)
      expect(s, `${m.name}: the page changed`).not.toBeNull()
      expect(s!.menus, `${m.name}: default context menu not prevented`).toEqual([true])
      seen[m.name] = s!.menu ?? []
      if (m.items.length === 0) expect(s!.menu, `${m.name}: no menu`).toBeNull()
      else expect(s!.menu, `${m.name}: Kor's menu items`).toEqual(m.items.map(tr))
      await settle(d)
    }
    test.info().annotations.push({ type: 'context menus', description: JSON.stringify(seen) })
    const end = await probe(d)
    expect(end?.marker).toBe(marker)
  } finally {
    await d.close()
    await served.stop().catch(() => undefined)
  }
})
