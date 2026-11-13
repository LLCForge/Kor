// Starting the real desktop app for a test and attaching a driver (shared by the desktop specs).
// Windows: Playwright over the WebView2 DevTools protocol; elsewhere (or `KOR_E2E_DRIVER=webdriver`)
// W3C WebDriver through `tauri-driver`.
import { chromium, type Browser, type Page } from '@playwright/test'
import { chmodSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { cdpDriver, until, webdriverDriver, type Driver } from './driver.ts'
import { CDP_PORT, appReport, closeDesktop, desktopBinary, launchDesktop, type Served } from './harness.ts'

export const USE_WEBDRIVER = process.env.KOR_E2E_DRIVER === 'webdriver' || process.platform !== 'win32'

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

export async function attach(served: Served): Promise<{ d: Driver; page?: Page }> {
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
