// Gerçek sağlayıcıyla elle tetiklenen izin denemesi (Venice; VENICE_API_KEY gerekir, az da olsa para harcar).
// 1) "Bu oturumda hep izin ver" sonrası aynı araç (Bash) farklı komutla bir daha sormaz.
// 2) Tur sürerken "İzinleri atla" seçilince bekleyen onay kendiliğinden geçer.
// Ayrıca Bash kartı açılınca `$ komut` satırı görünür. Ayar/veri klasörleri geçici ve yalıtılmış.
// KOR_E2E_CDP_PORT=9446 npx playwright test real-permission -c e2e/playwright.config.ts --project=audit
import { test, expect, chromium, type Browser, type Page } from '@playwright/test'
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import { mkdtempSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { until } from './driver.ts'
import { CDP_PORT, REPO, desktopBinary } from './harness.ts'

type Kor = { setWorkspacePath(d: string): Promise<unknown>; setPermissionMode(m: string): Promise<unknown> }

async function launch(): Promise<{ app: ChildProcess; browser: Browser; page: Page }> {
  const home = mkdtempSync(join(tmpdir(), 'kor-perm-'))
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
  await until(async () => { browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`); return true }, 60_000, 'cdp')
  let page: Page | undefined
  await until(async () => {
    for (const c of browser!.contexts().flatMap((x) => x.pages())) {
      if (await c.evaluate(() => document.querySelector('textarea.composer-input') !== null).catch(() => false)) { page = c; return true }
    }
    return false
  }, 90_000, 'main window')
  const p = page!
  await p.setViewportSize({ width: 1500, height: 950 })
  await p.evaluate(async (dir: string) => { await (window as unknown as { kor: Kor }).kor.setWorkspacePath(dir) }, REPO)
  await p.reload()
  await p.locator('textarea.composer-input').waitFor({ timeout: 60_000 })
  // An untrusted folder pins the mode to manual; trust it the way the user does.
  const trust = p.getByRole('button', { name: 'Bu klasöre güven' })
  if (await trust.count()) await trust.first().click()
  await p.evaluate(async () => { await (window as unknown as { kor: Kor }).kor.setPermissionMode('default') })
  return { app, browser: browser!, page: p }
}

function close(app: ChildProcess, browser: Browser | undefined): void {
  void browser?.close().catch(() => undefined)
  if (app.pid !== undefined) spawnSync('taskkill', ['/PID', String(app.pid), '/T', '/F'], { stdio: 'ignore' })
}

/** Type and send; true once the turn started (the stop button shows). */
async function send(p: Page, text: string): Promise<void> {
  for (let i = 0; i < 3; i++) {
    await p.waitForTimeout(1500)
    const box = p.locator('textarea.composer-input')
    await box.click()
    await box.fill(text)
    await box.press('Enter')
    try {
      await p.locator('button[aria-label="Durdur"]').waitFor({ timeout: 15_000 })
      return
    } catch { /* not started: try again */ }
  }
  throw new Error('message was not sent')
}

async function turnEnds(p: Page, maxMs: number, onTick: () => Promise<void>): Promise<void> {
  const started = Date.now()
  while (Date.now() - started < maxMs) {
    await onTick()
    if ((await p.locator('button[aria-label="Durdur"]').count()) === 0 && Date.now() - started > 5000) return
    await p.waitForTimeout(1000)
  }
  throw new Error('turn did not end')
}

test('always-allow covers the whole tool; skip-permissions answers a waiting card', async () => {
  test.skip(!process.env.VENICE_API_KEY, 'needs VENICE_API_KEY')
  test.setTimeout(15 * 60_000)

  // 1) Always allow.
  let run = await launch()
  try {
    const p = run.page
    await send(p, 'Bash aracıyla şu iki komutu AYRI AYRI, iki ayrı Bash çağrısıyla sırayla çalıştır: `git log -1 --oneline` ve sonra `git branch --show-current`. Sonunda iki çıktıyı yaz.')
    let asked = 0
    await turnEnds(p, 6 * 60_000, async () => {
      const always = p.getByRole('button', { name: 'Bu oturumda hep izin ver' })
      if (await always.count()) {
        asked += 1
        await always.first().click().catch(() => undefined)
        await p.waitForTimeout(1500)
      }
    })
    await p.screenshot({ path: join(REPO, 'docs', 'ui-audit', 'real-permission-always.png') })
    console.log('chat text:', (await p.locator('main').innerText()).replace(/\s+/g, ' ').slice(0, 1200))
    for (const g of await p.getByText(/araç kullanıldı/).all()) await g.click({ timeout: 3000 }).catch(() => undefined)
    const bashCards = await p.locator('.tool').filter({ hasText: 'Bash' }).count()
    console.log('always-allow: approvals asked =', asked, '· Bash cards =', bashCards)
    expect(bashCards).toBeGreaterThanOrEqual(1)
    expect(asked).toBe(1)
    // The Bash card opens on its command.
    // Finished tools fold into "N araç kullanıldı" groups: open them first.
    for (const g of await p.getByText(/araç kullanıldı/).all()) await g.click({ timeout: 3000 }).catch(() => undefined)
    await p.locator('.tool .tool-row').filter({ hasText: 'Bash' }).first().click({ timeout: 10_000 })
    const cmd = await p.locator('.tool-live-cmd').first().innerText({ timeout: 10_000 })
    console.log('first Bash card command line:', cmd)
    expect(cmd.startsWith('$ ')).toBe(true)
    await p.screenshot({ path: join(REPO, 'docs', 'ui-audit', 'real-permission-always.png') })
  } finally {
    close(run.app, run.browser)
  }

  // 2) Skip permissions while a card waits.
  run = await launch()
  try {
    const p = run.page
    await send(p, 'Bash aracıyla yalnızca `git status --short | head -5` komutunu çalıştır ve çıktıyı yaz.')
    await p.getByRole('button', { name: 'Bu oturumda hep izin ver' }).first().waitFor({ timeout: 5 * 60_000 })
    await p.evaluate(async () => { await (window as unknown as { kor: Kor }).kor.setPermissionMode('bypass') })
    // Nothing is clicked here: the waiting card must be answered by the mode switch alone.
    await turnEnds(p, 6 * 60_000, async () => undefined)
    for (const g of await p.getByText(/araç kullanıldı/).all()) await g.click({ timeout: 3000 }).catch(() => undefined)
    const result = await p.locator('.tool').filter({ hasText: 'Bash' }).first().innerText({ timeout: 10_000 })
    console.log('skip-permissions: Bash card =', result.replace(/\s+/g, ' ').slice(0, 200))
    expect(await p.getByRole('button', { name: 'Bu oturumda hep izin ver' }).count()).toBe(0)
    await p.screenshot({ path: join(REPO, 'docs', 'ui-audit', 'real-permission-bypass.png') })
  } finally {
    close(run.app, run.browser)
  }
})
