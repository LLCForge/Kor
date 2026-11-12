// §16.4 / §17.2 on the desktop surface (ADR 0061): every scenario in tests/e2e/scenarios against the real
// app and core with the mock provider, checked by the same Rust checker as the CLI and TUI; then every
// registered panel's screenshot (visual regression) in dark and light with reduced motion.
//
// Windows: Playwright attaches to WebView2 (CDP). Elsewhere `KOR_E2E_DRIVER=webdriver` drives the app
// through `tauri-driver` (start it first: `tauri-driver --port 4444`); screenshots are Windows-only.
import { test, expect, chromium, type Browser, type Page } from '@playwright/test'
import { chmodSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { cdpDriver, until, webdriverDriver, type Driver } from './driver.ts'
import { CDP_PORT, SCENARIOS, appReport, check, closeDesktop, desktopBinary, launchDesktop, serve, type Served } from './harness.ts'
import { costPanelText, openProject, runSteps } from './flow.ts'

const USE_WEBDRIVER = process.env.KOR_E2E_DRIVER === 'webdriver' || process.platform !== 'win32'

/** A script in the scenario's folder that starts the desktop binary with `served.env`. */
function launcher(served: Served): string {
  const bin = desktopBinary()
  if (process.platform === 'win32') {
    const file = join(served.root, 'kor-desktop.cmd')
    const vars = Object.entries(served.env).map(([k, v]) => `set "${k}=${v}"`)
    writeFileSync(file, ['@echo off', ...vars, `"${bin}" %*`, ''].join('\r\n'))
    return file
  }
  const q = (v: string) => `'${v.replaceAll("'", `'\\''`)}'`
  const vars = Object.entries(served.env).map(([k, v]) => `export ${k}=${q(v)}`)
  const file = join(served.root, 'kor-desktop.sh')
  writeFileSync(file, ['#!/bin/sh', ...vars, `exec ${q(bin)} "$@"`, ''].join('\n'))
  chmodSync(file, 0o755)
  return file
}

async function attach(served: Served): Promise<{ d: Driver; page?: Page }> {
  if (USE_WEBDRIVER) {
    // tauri-driver runs before the tests with its own environment, so the scenario's isolated home
    // reaches the app through a launcher script that sets it and starts the real binary.
    return { d: await webdriverDriver(launcher(served)) }
  }
  const app = launchDesktop(served)
  let browser: Browser | undefined
  await until(async () => {
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`)
    return true
  }, 60_000, 'the WebView2 debugging port').catch((e: unknown) => {
    throw new Error(`${e instanceof Error ? e.message : String(e)} (${appReport(app)})`)
  })
  const find = async (): Promise<Page> => {
    let page: Page | undefined
    await until(async () => {
      page = browser!.contexts().flatMap((c) => c.pages()).find((p) => !p.isClosed() && /index\.html|tauri\.localhost|^tauri:/.test(p.url()) && !/pip|popup/i.test(p.url()))
      return page !== undefined
    }, 60_000, 'the main window')
    return page!
  }
  const page = await find()
  return {
    page,
    d: cdpDriver(page, async () => {
      await browser!.close().catch(() => undefined)
      closeDesktop(app)
    }, find),
  }
}

const files = readdirSync(SCENARIOS).filter((f) => f.endsWith('.yaml')).sort()

for (const file of files) {
  const text = readFileSync(join(SCENARIOS, file), 'utf8')
  // `surfaces:` lists where a scenario runs; all three when absent.
  const surfaces = /^surfaces:\s*\[([^\]]*)\]/m.exec(text)?.[1]
  if (surfaces !== undefined && !surfaces.includes('desktop')) continue
  test(`scenario ${file}`, async () => {
    const path = join(SCENARIOS, file)
    const served = await serve(path)
    const { d } = await attach(served)
    try {
      await openProject(d, served.project)
      let output = await runSteps(d, served)
      if (served.scenario.steps.some((s) => s.prompt !== undefined)) output += await costPanelText(d)
      await d.close()
      await served.stop()
      const outcome = check(served, path, output)
      expect(outcome.problems, outcome.problems.join('\n')).toEqual([])
    } finally {
      await d.close()
      await served.stop().catch(() => undefined)
    }
  })
}

/** The registered panels (renderer/panels/registry.ts), opened through the command palette by label. */
function panels(): Array<{ id: string; label: string }> {
  const dir = join(SCENARIOS, '../../../frontend/src/desktop/renderer/panels')
  const reg = readFileSync(join(dir, 'registry.ts'), 'utf8')
  const ids = [...reg.matchAll(/import \{ panel as (\w+) \} from '\.\/([\w-]+)\/index\.tsx'/g)].map((m) => m[2]!)
  return ids.map((id) => {
    const src = readFileSync(join(dir, id, 'index.tsx'), 'utf8')
    return { id, label: /label:\s*'([^']+)'/.exec(src)?.[1] ?? id }
  })
}

test('every panel, dark and light, reduced motion (visual regression)', async () => {
  test.skip(USE_WEBDRIVER, 'screenshots need the WebView2 DevTools protocol (Windows)')
  const served = await serve(join(SCENARIOS, 'task-approve-write.yaml'))
  const { d, page } = await attach(served)
  try {
    await openProject(d, served.project)
    for (const scheme of ['dark', 'light'] as const) {
      await page!.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' })
      for (const p of panels()) {
        await d.keys('Control+k')
        await d.type(p.label)
        await d.keys('Enter')
        await page!.waitForTimeout(800)
        // Content that changes on its own (clocks, the temp project path) is masked.
        await expect(page!).toHaveScreenshot(`panel-${p.id}-${scheme}.png`, {
          mask: [page!.locator('time, [data-volatile], code[translate="no"]')],
          maxDiffPixelRatio: 0.001,
        })
      }
    }
  } finally {
    await d.close()
    await served.stop().catch(() => undefined)
  }
})
