// §16.4 fourth surface (§18.3, §18.4, ADR 0058/0061): the built Chrome extension's side panel in Playwright's
// Chromium (`--load-extension`, persistent profile), paired with the real desktop app over the extension
// bridge (ws://127.0.0.1:49201), running the scenario catalogue with the mock provider. The desktop owns the
// sessions the side panel drives (src-tauri/src/chrome_bridge.rs), so a run is checked against the desktop's
// core trace with the same Rust checker as the other surfaces (results: test-results/e2e/<name>-sidepanel.json).
//
// - "side panel loads" needs only the built extension (`npm run build:extension`): the service worker
//   starts with the manifest's fixed id and the panel renders its setup (unpaired) view.
// - The scenarios need, in addition, the desktop app and helpers the desktop project uses, port 49201
//   free, and `KOR_E2E_SIDEPANEL=1`: pairing stores the new key's SHA-256 in the OS keychain entry
//   `kor / chrome-bridge:keys` (KeychainStore, no override), so they are opt-in outside CI. Linux runners
//   have no Secret Service, where pairing cannot be stored: the CI job runs them on Windows.
//
// Which scenarios run: those for the desktop surface whose steps the panel can express: prompts (Chat
// composer), approvals (the approval card's buttons) and the `command` steps in PANEL_OPS (the panel's own
// core requests). The rest are listed as skipped with the step that has no side-panel equivalent.
import { test, expect, chromium, type BrowserContext, type Page } from '@playwright/test'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readdirSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { until } from './driver.ts'
import { attach } from './app.ts'
import { FRONTEND, SCENARIOS, check, korBinary, serve, type Served, type Step } from './harness.ts'
import { openProject } from './flow.ts'

const DIST = join(FRONTEND, 'extension/dist/chrome')
/** Fixed by the manifest `key` (kor_platform::native_messaging::CHROME_EXTENSION_ID). */
const EXTENSION_ID = 'iokjlmpngkhnhndnklpildpdfpfdjddh'

/** CLI `command` steps → the side panel's core request (`client(op)` in panel/bus.ts). */
const PANEL_OPS: Record<string, string> = {
  'kor cost': 'cost',
  'kor sessions': 'sessions',
}

async function openPanel(): Promise<{ ctx: BrowserContext; panel: Page; id: string }> {
  // KOR_BROWSER: another Chromium / Chrome for Testing (as the `real_extension` Rust test takes it).
  const exe = process.env.KOR_BROWSER
  const ctx = await chromium.launchPersistentContext(mkdtempSync(join(tmpdir(), 'kor-e2e-chrome-')), {
    // `chromium` (not the headless shell) runs extensions, headless included.
    ...(exe ? { executablePath: exe } : { channel: 'chromium' }),
    headless: process.env.KOR_E2E_HEADED !== '1',
    // Chrome 137+ branded builds ignore --load-extension unless this feature is off.
    args: [`--disable-extensions-except=${DIST}`, `--load-extension=${DIST}`, '--disable-features=DisableLoadExtensionCommandLineSwitch'],
  })
  const sw = ctx.serviceWorkers().find((w) => w.url().endsWith('/background.js')) ?? (await ctx.waitForEvent('serviceworker', { timeout: 30_000 }))
  const id = new URL(sw.url()).host
  const panel = await ctx.newPage()
  await panel.goto(`chrome-extension://${id}/sidepanel.html`)
  return { ctx, panel, id }
}

/** A core request through the extension, as the panel sends it (`kor-panel` port → background → bridge). */
async function panelClient(panel: Page, op: string, body: Record<string, unknown> = {}): Promise<unknown> {
  return panel.evaluate(
    ({ op, body }) =>
      new Promise((ok, fail) => {
        const rt = (globalThis as unknown as { chrome: { runtime: { connect(i: { name: string }): { onMessage: { addListener(f: (m: Record<string, unknown>) => void): void }; postMessage(m: unknown): void; disconnect(): void } } } }).chrome.runtime
        const port = rt.connect({ name: 'kor-panel' })
        const rid = 1_000_000 + Math.floor(Math.random() * 1_000_000)
        port.onMessage.addListener((m) => {
          if (m.kind !== 'res' || m.rid !== rid) return
          port.disconnect()
          const r = m.result as Record<string, unknown> | null
          if (m.ok !== true) fail(new Error(String(m.error)))
          else if (r && typeof r === 'object' && typeof r.error === 'string') fail(new Error(r.error))
          else ok(r)
        })
        port.postMessage({ op: 'client', body: { ...body, op }, rid })
      }),
    { op, body },
  )
}

/** The turn is over: nothing running in the desktop core for two polls. */
async function turnEnd(panel: Page): Promise<void> {
  let quiet = 0
  await until(async () => {
    const s = (await panelClient(panel, 'status')) as { running: unknown[] }
    quiet = s.running.length === 0 ? quiet + 1 : 0
    return quiet >= 2
  }, 120_000, 'the turn to end')
}

async function pair(panel: Page, served: Served): Promise<void> {
  const r = spawnSync(korBinary(), ['chrome', 'pair'], { env: { ...process.env, ...served.env }, encoding: 'utf8', cwd: served.project })
  const code = /\b([A-Z0-9]{4}-[A-Z0-9]{4})\b/.exec(r.stdout)?.[1]
  if (!code) throw new Error(`kor chrome pair gave no code: ${r.stdout}${r.stderr}`)
  // The desktop starts the bridge in the background: the panel reaches it once it listens.
  await until(async () => (await panel.evaluate(() => document.querySelector('.setup')?.textContent ?? '')).length > 0, 30_000, 'the setup view')
  await panel.locator('.setup input[type="text"]').first().fill(code)
  await panel.locator('.setup button.primary').click()
  await panel.locator('.chat').waitFor({ timeout: 30_000 })
}

async function runSteps(panel: Page, steps: Step[]): Promise<string> {
  let output = ''
  const textarea = panel.locator('.chat footer textarea')
  const card = panel.locator('[data-kor-cmd="client.approve"]').last()
  for (let i = 0; i < steps.length; i++) {
    const st = steps[i]!
    if (st.prompt !== undefined) {
      await textarea.fill(st.prompt)
      await textarea.press('Enter')
      if (steps[i + 1]?.approve !== undefined) await card.waitFor({ timeout: 120_000 })
      else {
        await panel.waitForTimeout(400)
        await turnEnd(panel)
      }
    } else if (st.approve !== undefined) {
      // İzin ver (primary) · Her zaman · Reddet, in the card's button row.
      const buttons = card.locator('.row').last().locator('button')
      await (st.approve === 'yes' ? buttons.first() : st.approve === 'no' ? buttons.last() : buttons.nth(1)).click()
      await turnEnd(panel)
    } else if (st.command !== undefined) {
      const sessions = (await panelClient(panel, 'sessions')) as Array<{ id: string }>
      output += `${JSON.stringify(await panelClient(panel, PANEL_OPS[st.command]!, { sessionId: sessions[0]?.id }))}\n`
    }
  }
  return output
}

test('side panel loads: fixed extension id, setup view before pairing', async () => {
  test.skip(!existsSync(join(DIST, 'manifest.json')), 'build the extension first: npm run build:extension')
  const { ctx, panel, id } = await openPanel()
  try {
    expect(id).toBe(EXTENSION_ID)
    await panel.locator('.setup').waitFor({ timeout: 15_000 })
    // A fresh profile has no pairing key: the pairing field is offered.
    await expect(panel.locator('.setup input[type="text"]').first()).toBeVisible()
    expect(await panel.title()).toMatch(/Kor/)

    // §21.2 / §21.3 on the panel page (webview-lock.js): Kor's own menu in a text field (clipboard items
    // only in the panel), no menu on plain text, no reload from F5 / Ctrl+R. Chrome's own browser-level
    // shortcuts (Ctrl+T, F11, Ctrl+Shift+I…) never reach the page and are listed in ADR 0053 instead.
    await panel.evaluate(() => {
      const w = window as unknown as { __korMarker: number; __korMenus: boolean[] }
      w.__korMarker = 1
      w.__korMenus = []
      addEventListener('contextmenu', (e) => { setTimeout(() => w.__korMenus.push(e.defaultPrevented), 0) }, true)
    })
    await panel.locator('.setup input[type="text"]').first().click({ button: 'right' })
    await expect(panel.locator('.kor-context-menu [role="menuitem"]')).toHaveText(['Kes', 'Kopyala', 'Yapıştır', 'Tümünü seç'])
    await panel.keyboard.press('Escape')
    await expect(panel.locator('.kor-context-menu')).toHaveCount(0)
    await panel.locator('.setup p').first().click({ button: 'right' })
    await panel.waitForTimeout(300)
    await expect(panel.locator('.kor-context-menu')).toHaveCount(0)
    await panel.locator('.setup p').first().click()
    for (const k of ['F5', 'Control+R', 'Control+Shift+R', 'Alt+ArrowLeft', 'Backspace']) await panel.keyboard.press(k)
    await panel.waitForTimeout(500)
    expect(await panel.evaluate(() => (window as unknown as { __korMarker?: number }).__korMarker), 'the panel reloaded').toBe(1)
    expect(await panel.evaluate(() => (window as unknown as { __korMenus: boolean[] }).__korMenus)).toEqual([true, true])
  } finally {
    await ctx.close()
  }
})

const files = readdirSync(SCENARIOS).filter((f) => f.endsWith('.yaml')).sort()

for (const file of files) {
  const text = readFileSync(join(SCENARIOS, file), 'utf8')
  const surfaces = /^surfaces:\s*\[([^\]]*)\]/m.exec(text)?.[1]
  const commands = [...text.matchAll(/^\s+- command:\s*"([^"]+)"/gm)].map((m) => m[1]!)
  // Choice cards (`choose:`) and media runs (pasted images, galleries) have no side-panel control yet.
  const missing = commands.find((c) => !(c in PANEL_OPS)) ?? (/^\s+- choose:/m.test(text) ? 'choose' : /^\s+media:\s*true/m.test(text) ? 'media: true' : undefined)
  test(`scenario ${file} (side panel)`, async () => {
    test.skip(surfaces !== undefined && !surfaces.includes('desktop'), 'not a desktop-surface scenario (the side panel drives the desktop core)')
    test.skip(missing !== undefined, `step \`${missing}\` has no side-panel request`)
    test.skip(process.env.KOR_E2E_SIDEPANEL !== '1', 'set KOR_E2E_SIDEPANEL=1 (pairs through the OS keychain; needs the desktop app and port 49201)')
    test.skip(!existsSync(join(DIST, 'manifest.json')), 'build the extension first: npm run build:extension')
    const path = join(SCENARIOS, file)
    const served = await serve(path)
    const { d } = await attach(served)
    let ctx: BrowserContext | undefined
    try {
      // The desktop's workspace and permission mode, as in the desktop run; then the panel takes over.
      await openProject(d, served.project)
      const opened = await openPanel()
      ctx = opened.ctx
      await pair(opened.panel, served)
      let output = await runSteps(opened.panel, served.scenario.steps)
      if (served.scenario.steps.some((s) => s.prompt !== undefined)) output += `${JSON.stringify(await panelClient(opened.panel, 'cost'))}\n`
      output += await opened.panel.locator('.chat .log').innerText()
      await ctx.close()
      await d.close()
      await served.stop()
      const outcome = check(served, path, output, 'sidepanel')
      expect(outcome.problems, outcome.problems.join('\n')).toEqual([])
    } finally {
      await ctx?.close().catch(() => undefined)
      await d.close()
      await served.stop().catch(() => undefined)
    }
  })
}
