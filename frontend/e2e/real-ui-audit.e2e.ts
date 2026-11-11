// Gerçek sağlayıcıyla elle tetiklenen deneme (Venice; ortamda VENICE_API_KEY olmalı, para harcar):
// kor-agent klasöründe "projeyi açıkla", ardından proje grafı. Ayar/veri klasörleri geçici ve yalıtılmış.
// KOR_E2E_CDP_PORT=9444 npx playwright test real-ui-audit -c e2e/playwright.config.ts --project=audit
import { test, chromium, type Browser, type Page } from '@playwright/test'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { until } from './driver.ts'
import { CDP_PORT, REPO, desktopBinary } from './harness.ts'

test('real provider: explain the project, then the graph', async () => {
  test.skip(!process.env.VENICE_API_KEY, 'needs VENICE_API_KEY')
  test.setTimeout(30 * 60_000)
  // KOR_REAL_HOME reuses an earlier run's folders (graph already built).
  const home = process.env.KOR_REAL_HOME ?? mkdtempSync(join(tmpdir(), 'kor-real-'))
  for (const d of ['home', 'config', 'cache']) mkdirSync(join(home, d), { recursive: true })
  const env = {
    ...(process.env as Record<string, string>),
    KOR_HOME: join(home, 'home'),
    KOR_CONFIG_DIR: join(home, 'config'),
    KOR_CACHE_DIR: join(home, 'cache'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
  }
  const app = spawn(desktopBinary(), [], { env, stdio: 'ignore', cwd: REPO })
  let browser: Browser | undefined
  const shots = join(REPO, 'docs', 'ui-audit')
  try {
    await until(async () => { browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`); return true }, 60_000, 'cdp')
    let page: Page | undefined
    await until(async () => {
      for (const c of browser!.contexts().flatMap((x) => x.pages())) {
        if (await c.evaluate(() => document.querySelector('textarea.composer-input') !== null).catch(() => false)) { page = c; return true }
      }
      return false
    }, 90_000, 'main window')
    const p = page!
    const errors: string[] = []
    p.on('pageerror', (e) => errors.push(String(e)))
    await p.setViewportSize({ width: 1600, height: 1000 })
    await p.evaluate(async (dir: string) => { await (window as unknown as { kor: { setWorkspacePath(d: string): Promise<unknown> } }).kor.setWorkspacePath(dir) }, REPO)
    await p.reload()
    const box = p.locator('textarea.composer-input')
    await box.waitFor({ timeout: 60_000 })
    if (!process.env.KOR_REAL_GRAPH_ONLY) {
    await box.fill('Bu projeyi kısaca açıkla: ne işe yarıyor, klasör yapısı nasıl? Dosyaları okuyabilirsin, hiçbir şeyi değiştirme.')
    await box.press('Enter')
    // A new chat shows up in the sidebar while its first turn runs.
    const listed = await p.locator('.session-row').count()
    console.log('sidebar sessions after send:', listed)
    // Approve read-only tools if asked; wait for the turn to end (stop button gone).
    const started = Date.now()
    while (Date.now() - started < 15 * 60_000) {
      const approve = p.getByRole('button', { name: /Bu oturumda hep izin ver/ })
      if (await approve.count()) await approve.first().click().catch(() => undefined)
      if ((await p.locator('button[aria-label="Durdur"]').count()) === 0 && Date.now() - started > 5000) break
      await p.waitForTimeout(2000)
    }
    console.log('turn seconds:', Math.round((Date.now() - started) / 1000))
    await p.screenshot({ path: join(shots, 'real-explain.png') })
    const answer = await p.locator('.markdown, .assistant-text, [data-role="assistant"]').last().innerText().catch(() => '')
    console.log('answer chars:', answer.length, answer.slice(0, 600))
    }
    await p.evaluate(() => window.dispatchEvent(new CustomEvent('kor:panel-open', { detail: 'graph' })))
    await p.locator('.kg-side').waitFor({ timeout: 10 * 60_000 })
    await p.waitForTimeout(10_000)
    await p.screenshot({ path: join(shots, 'real-graph.png') })
    console.log('page errors:', errors.length ? errors.join(' | ') : 'none')
  } finally {
    await browser?.close().catch(() => undefined)
    if (app.pid !== undefined) spawnSync('taskkill', ['/PID', String(app.pid), '/T', '/F'], { stdio: 'ignore' })
  }
})
