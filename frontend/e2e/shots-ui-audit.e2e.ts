// Design review captures (not a gate): the real desktop app against the mock provider at three
// window sizes — home, a chat with a tool card, settings. Output folder: KOR_SHOTS_DIR.
// KOR_SHOTS_DIR=<dir> KOR_E2E_CDP_PORT=9447 npx playwright test shots-ui-audit -c e2e/playwright.config.ts --project=audit
import { test, chromium, type Browser, type Page } from '@playwright/test'
import { mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { until } from './driver.ts'
import { CDP_PORT, SCENARIOS, closeDesktop, launchDesktop, serve } from './harness.ts'

const SIZES = [
  { name: 'narrow', width: 900, height: 650 },
  { name: 'laptop', width: 1366, height: 768 },
  { name: 'large', width: 2560, height: 1440 },
] as const

test('design review captures', async () => {
  test.skip(!process.env.KOR_SHOTS_DIR, 'set KOR_SHOTS_DIR')
  test.setTimeout(10 * 60_000)
  const out = process.env.KOR_SHOTS_DIR!
  mkdirSync(out, { recursive: true })
  const served = await serve(join(SCENARIOS, 'task-approve-write.yaml'))
  const app = launchDesktop(served)
  let browser: Browser | undefined
  try {
    await until(async () => { browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`); return true }, 60_000, 'cdp')
    let page: Page | undefined
    await until(async () => { page = browser!.contexts().flatMap((c) => c.pages()).find((p) => /index\.html|tauri\.localhost/.test(p.url())); return page !== undefined }, 60_000, 'main window')
    const p = page!
    await p.locator('textarea.composer-input').waitFor({ timeout: 90_000 })
    await p.evaluate(async (dir: string) => { await (window as unknown as { kor: { setWorkspacePath(d: string): Promise<unknown> } }).kor.setWorkspacePath(dir) }, served.project)
    await p.reload()
    await p.locator('textarea.composer-input').waitFor({ timeout: 60_000 })
    const shot = async (name: string) => {
      for (const s of SIZES) {
        await p.setViewportSize(s)
        await p.waitForTimeout(500)
        await p.screenshot({ path: join(out, `${name}-${s.name}.png`) })
      }
    }
    await shot('1-home')
    const box = p.locator('textarea.composer-input')
    await box.click()
    await box.fill('notes.md adında bir not dosyası oluştur')
    await box.press('Enter')
    // A keyword match may offer a `kor` command first; the message goes to the agent.
    const toAgent = p.getByRole('button', { name: 'Ajana gönder' })
    await toAgent.waitFor({ timeout: 4000 }).then(() => toAgent.click()).catch(() => undefined)
    await p.getByRole('button', { name: /İzin ver|Kabul/ }).first().waitFor({ timeout: 30_000 }).catch(() => undefined)
    await shot('2-approval')
    await p.getByRole('button', { name: /İzin ver|Kabul/ }).first().click({ timeout: 5000 }).catch(() => undefined)
    await p.waitForTimeout(4000)
    await shot('3-chat')
    await p.evaluate(() => window.dispatchEvent(new CustomEvent('kor:panel-open', { detail: 'cost' })))
    await p.waitForTimeout(800)
    await shot('4-panel')
    await p.keyboard.press('Control+,')
    await p.waitForTimeout(800)
    await shot('5-settings')
  } finally {
    await browser?.close().catch(() => undefined)
    closeDesktop(app)
    await served.stop().catch(() => undefined)
  }
})
