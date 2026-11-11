// Responsive guard: the built renderer in the real desktop app (WebView2 over CDP) at the window minimum
// (800×600) and at 1920×1080 must not scroll or overflow horizontally — the main layout, the open settings
// dialog and the command palette. Runs with the `desktop` project (Windows CI); `--project desktop`.
import { test, expect, chromium, type Browser, type Page } from '@playwright/test'
import { join } from 'node:path'
import { until } from './driver.ts'
import { CDP_PORT, SCENARIOS, closeDesktop, launchDesktop, serve } from './harness.ts'

const SIZES = [
  { width: 800, height: 600 },
  { width: 1920, height: 1080 },
] as const

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
    if (el.scrollWidth > el.clientWidth + 1) out.push(`${cls} content ${el.scrollWidth} > ${el.clientWidth}`)
    if (r.right > vw + 1 || r.left < -1) out.push(`${cls} outside the window [${Math.round(r.left)}, ${Math.round(r.right)}]`)
  }
  return out
}

test('no horizontal overflow at 800×600 and 1920×1080', async () => {
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
    }
    expect(problems).toEqual([])
  } finally {
    await browser?.close().catch(() => undefined)
    closeDesktop(app)
    await served.stop().catch(() => undefined)
  }
})
