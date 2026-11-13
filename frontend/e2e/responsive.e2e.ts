// Responsive guard: the built renderer in the real desktop app (WebView2 over CDP) at the window minimum
// (800×600), at 1920×1080 and on large screens (2560×1440, 3840×2160) must not scroll or overflow
// horizontally — the main layout, the open settings dialog, the command palette and every side-panel
// view (the side panel itself is narrower than 800 px, so this covers the narrow layouts too).
// Runs with the `desktop` project (Windows CI); `--project desktop`.
import { test, expect, chromium, type Browser, type Page } from '@playwright/test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { until } from './driver.ts'
import { CDP_PORT, REPO, SCENARIOS, closeDesktop, launchDesktop, serve } from './harness.ts'

const SIZES = [
  { width: 800, height: 600 },
  { width: 1920, height: 1080 },
  { width: 2560, height: 1440 },
  { width: 3840, height: 2160 },
] as const

/** Side-panel view ids, read from each panel's `index.ts(x)` (`id: 'graph'`). */
function panelIds(): string[] {
  const dir = join(REPO, 'frontend', 'src', 'desktop', 'renderer', 'panels')
  const ids = new Set<string>()
  for (const d of readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory())) {
    for (const f of ['index.ts', 'index.tsx']) {
      try {
        const m = /^\s*id: '([a-z-]+)',/m.exec(readFileSync(join(dir, d.name, f), 'utf8'))
        if (m) ids.add(m[1]!)
      } catch { /* no such file */ }
    }
  }
  return [...ids]
}

/** Runs in the page: elements inside the open side panel that reach past its edges. */
function panelOverflow(): string[] {
  const panel = document.querySelector<HTMLElement>('.side-panel')
  if (!panel) return ['no side panel']
  const box = panel.getBoundingClientRect()
  const out: string[] = []
  for (const el of Array.from(panel.querySelectorAll<HTMLElement>('*'))) {
    const r = el.getBoundingClientRect()
    if (r.width === 0 || r.height === 0) continue
    // The resize handle straddles the panel's edge on purpose.
    if (el.closest('[class*="resizer"]')) continue
    // Content inside a scroller is meant to be scrolled to.
    let p = el.parentElement
    let scrolled = false
    while (p && p !== panel) {
      const s = getComputedStyle(p).overflowX
      if (s === 'auto' || s === 'scroll' || s === 'hidden') { scrolled = true; break }
      p = p.parentElement
    }
    if (scrolled) continue
    if (r.right > box.right + 1 || r.left < box.left - 1) {
      const cls = (el.getAttribute('class') ?? el.tagName).split(/\s+/)[0]
      out.push(`${cls} [${Math.round(r.left)}, ${Math.round(r.right)}] outside [${Math.round(box.left)}, ${Math.round(box.right)}]`)
    }
    if (out.length > 5) break
  }
  return out
}

/** Runs in the page: layout containers whose content is wider than they are, and the document scroll. */
function overflow(): string[] {
  const out: string[] = []
  const vw = window.innerWidth
  if (document.documentElement.scrollWidth > vw + 1) out.push(`document ${document.documentElement.scrollWidth} > ${vw}`)
  const layout = '.app, .titlebar, .app-body, .rail, .session-grid, .main, .main-header, .viewport, .messages, .viewport-footer, .composer, .side-panel, dialog[open], dialog[open] .g7-settings-body, dialog[open] .g7-settings-content'
  for (const el of Array.from(document.querySelectorAll<HTMLElement>(layout))) {
    const r = el.getBoundingClientRect()
    if (r.width === 0 || r.height === 0) continue
    const cls = (el.getAttribute('class') ?? el.tagName).split(/\s+/)[0]
    if (el.scrollWidth > el.clientWidth + 1) {
      // Name the widest child so the report says what overflows.
      let widest = ''
      let w = 0
      for (const c of Array.from(el.querySelectorAll<HTMLElement>('*'))) {
        const cw = c.getBoundingClientRect().right - el.getBoundingClientRect().left
        if (cw > w) { w = cw; widest = (c.getAttribute('class') ?? c.tagName).split(' ')[0] ?? '' }
      }
      out.push(`${cls} content ${el.scrollWidth} > ${el.clientWidth} (${widest})`)
    }
    if (r.right > vw + 1 || r.left < -1) out.push(`${cls} outside the window [${Math.round(r.left)}, ${Math.round(r.right)}]`)
  }
  return out
}

test('no horizontal overflow from 800×600 to 3840×2160, side panels included', async () => {
  test.setTimeout(20 * 60_000)
  test.skip(process.platform !== 'win32' || process.env.KOR_E2E_DRIVER === 'webdriver', 'attaches to WebView2 (Windows)')
  const served = await serve(join(SCENARIOS, 'task-approve-write.yaml'))
  const app = launchDesktop(served)
  let browser: Browser | undefined
  try {
    await until(async () => { browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`); return true }, 60_000, 'WebView2 debugging port')
    let page: Page | undefined
    await until(async () => { page = browser!.contexts().flatMap((c) => c.pages()).find((p) => /index\.html|tauri\.localhost/.test(p.url())); return page !== undefined }, 60_000, 'main window')
    const p = page!
    await p.locator('textarea.composer-input').waitFor({ timeout: 90_000 })
    await p.evaluate(async (dir: string) => { await (window as unknown as { kor: { setWorkspacePath(d: string): Promise<unknown> } }).kor.setWorkspacePath(dir) }, served.project)
    await p.reload()
    await p.locator('textarea.composer-input').waitFor({ timeout: 60_000 })

    const problems: string[] = []
    for (const size of SIZES) {
      await p.setViewportSize(size)
      await p.keyboard.press('Escape')
      await p.waitForTimeout(500)
      problems.push(...(await p.evaluate(overflow)).map((x) => `${size.width}×${size.height} home: ${x}`))
      await p.keyboard.press('Control+k')
      await p.waitForTimeout(400)
      problems.push(...(await p.evaluate(overflow)).map((x) => `${size.width}×${size.height} palette: ${x}`))
      await p.keyboard.press('Escape')
      await p.keyboard.press('Control+,')
      await p.waitForTimeout(600)
      problems.push(...(await p.evaluate(overflow)).map((x) => `${size.width}×${size.height} settings: ${x}`))
      await p.keyboard.press('Escape')
      for (const id of panelIds()) {
        await p.evaluate((v: string) => window.dispatchEvent(new CustomEvent('kor:panel-open', { detail: v })), id)
        await p.waitForTimeout(350)
        problems.push(...(await p.evaluate(overflow)).map((x) => `${size.width}×${size.height} ${id}: ${x}`))
        // Configuration views (view-catalog.ts SETTINGS_VIEWS) open as a Settings section, not in the side panel.
        if (await p.locator('dialog[open]').count()) await p.keyboard.press('Escape')
        else problems.push(...(await p.evaluate(panelOverflow)).map((x) => `${size.width}×${size.height} ${id}: ${x}`))
      }
      await p.screenshot({ path: join(REPO, 'docs', 'ui-audit', `responsive-${size.width}.png`) })
    }
    expect(problems).toEqual([])
  } finally {
    await browser?.close().catch(() => undefined)
    closeDesktop(app)
    await served.stop().catch(() => undefined)
  }
})
