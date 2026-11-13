// One small driver interface over the two automation protocols the desktop app supports:
// Playwright/CDP (WebView2 on Windows) and W3C WebDriver (`tauri-driver`: WebKitGTK on Linux, Edge
// WebDriver on Windows). Scenario flows and the panel sweep are written once against it.
import type { Page } from '@playwright/test'

export interface Driver {
  readonly kind: 'cdp' | 'webdriver'
  /** Runs `fn` in the page (serialized: no closures) and returns its JSON result. The app's CSP forbids
   *  `eval`, so code is never built from strings inside the page. */
  run<T, A extends unknown[]>(fn: (...args: A) => T | Promise<T>, ...args: A): Promise<T>
  click(selector: string): Promise<void>
  /** A real right click (secondary button) on the element's centre. */
  contextClick(selector: string): Promise<void>
  fill(selector: string, text: string): Promise<void>
  press(selector: string, key: 'Enter' | 'Escape'): Promise<void>
  waitFor(selector: string, timeoutMs?: number): Promise<void>
  text(selector: string): Promise<string>
  reload(): Promise<void>
  /** A key chord on the focused element (`Control+k`), or text typed into it. */
  keys(chord: string): Promise<void>
  type(text: string): Promise<void>
  close(): Promise<void>
}

export async function until(cond: () => Promise<boolean>, timeoutMs: number, what: string): Promise<void> {
  const end = Date.now() + timeoutMs
  while (Date.now() < end) {
    if (await cond().catch(() => false)) return
    await new Promise((r) => setTimeout(r, 200))
  }
  throw new Error(`timed out waiting for ${what}`)
}

/** `find` re-acquires the main window: WebView2 replaces the page target on a reload. */
export function cdpDriver(first: Page, close: () => Promise<void>, find?: () => Promise<Page>): Driver {
  let page = first
  return {
    kind: 'cdp',
    run: async <T, A extends unknown[]>(fn: (...args: A) => T | Promise<T>, ...args: A) =>
      (await page.evaluate(`(${fn.toString()})(...${JSON.stringify(args)})`)) as T,
    click: (s) => page.locator(s).first().click(),
    contextClick: (s) => page.locator(s).first().click({ button: 'right' }),
    fill: (s, t) => page.locator(s).first().fill(t),
    press: (s, k) => page.locator(s).first().press(k),
    waitFor: (s, t = 60_000) => page.locator(s).first().waitFor({ timeout: t }),
    text: (s) => page.locator(s).first().innerText(),
    reload: async () => {
      await page.reload().catch(() => undefined)
      await new Promise((r) => setTimeout(r, 1500))
      if (find) page = await find()
    },
    keys: (c) => page.keyboard.press(c),
    type: (t) => page.keyboard.type(t),
    close,
  }
}

/** Minimal W3C WebDriver client over fetch (no extra dependency) for `tauri-driver` on port 4444. */
export async function webdriverDriver(application: string, port = 4444): Promise<Driver> {
  const base = `http://127.0.0.1:${port}`
  const call = async (method: string, path: string, body?: unknown) => {
    // Every call is bounded: a hung driver fails the step instead of the whole 10-minute test.
    const r = await fetch(`${base}${path}`, {
      method,
      headers: { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(120_000),
    })
    const j = (await r.json()) as { value: unknown }
    if (!r.ok) throw new Error(`WebDriver ${method} ${path}: ${JSON.stringify(j.value)}`)
    return j.value
  }
  const session = (await call('POST', '/session', { capabilities: { alwaysMatch: { 'tauri:options': { application } } } })) as { sessionId: string }
  const sid = `/session/${session.sessionId}`
  let closed = false
  const find = async (selector: string) => {
    const v = (await call('POST', `${sid}/element`, { using: 'css selector', value: selector })) as Record<string, string>
    return Object.values(v)[0]!
  }
  const d: Driver = {
    kind: 'webdriver',
    run: async <T, A extends unknown[]>(fn: (...args: A) => T | Promise<T>, ...args: A) =>
      (await call('POST', `${sid}/execute/async`, {
        script: `const done = arguments[arguments.length - 1]; Promise.resolve((${fn.toString()})(...Array.from(arguments).slice(0, -1))).then((v) => done({ ok: v }), (e) => done({ err: String(e && e.message || e) }))`,
        args,
      }).then((v) => {
        const r = v as { ok?: T; err?: string }
        if (r.err !== undefined) throw new Error(r.err)
        return r.ok as T
      })),
    click: async (s) => {
      try {
        await call('POST', `${sid}/element/${await find(s)}/click`, {})
      } catch (e) {
        // A toast or a closing panel can sit over the button for a moment: the element's own click.
        if (!String(e).includes('click intercepted')) throw e
        await call('POST', `${sid}/execute/sync`, { script: 'document.querySelector(arguments[0]).click()', args: [s] })
      }
    },
    contextClick: async (s) => {
      const origin = { 'element-6066-11e4-a52e-4f735466cecf': await find(s) }
      const actions = [{ type: 'pointerMove', origin, x: 0, y: 0 }, { type: 'pointerDown', button: 2 }, { type: 'pointerUp', button: 2 }]
      await call('POST', `${sid}/actions`, { actions: [{ type: 'pointer', id: 'mouse', parameters: { pointerType: 'mouse' }, actions }] })
      await call('DELETE', `${sid}/actions`)
    },
    fill: async (s, t) => {
      const id = await find(s)
      await call('POST', `${sid}/element/${id}/clear`, {})
      await call('POST', `${sid}/element/${id}/value`, { text: t })
    },
    press: async (s, k) => { await call('POST', `${sid}/element/${await find(s)}/value`, { text: k === 'Enter' ? '\uE007' : '\uE00C' }) },
    waitFor: (s, t = 60_000) => until(async () => (await call('POST', `${sid}/elements`, { using: 'css selector', value: s }) as unknown[]).length > 0, t, s),
    text: async (s) => (await call('GET', `${sid}/element/${await find(s)}/text`)) as string,
    reload: async () => { await call('POST', `${sid}/refresh`, {}) },
    keys: async (chord) => {
      // W3C key codes; Playwright's names for the printable keys (`Equal`) map to the character.
      const code: Record<string, string> = {
        Control: '\uE009', Shift: '\uE008', Alt: '\uE00A', Meta: '\uE03D', Enter: '\uE007', Escape: '\uE00C', Tab: '\uE004',
        Backspace: '\uE003', Delete: '\uE017', ArrowLeft: '\uE012', ArrowRight: '\uE014',
        F5: '\uE035', F11: '\uE03B', F12: '\uE03C', Equal: '=', Minus: '-', Digit0: '0',
      }
      const parts = chord.split('+').map((k) => code[k] ?? k)
      const actions = [...parts.map((value) => ({ type: 'keyDown', value })), ...parts.reverse().map((value) => ({ type: 'keyUp', value }))]
      await call('POST', `${sid}/actions`, { actions: [{ type: 'key', id: 'kbd', actions }] })
      await call('DELETE', `${sid}/actions`)
    },
    type: async (t) => {
      const actions = [...t].flatMap((value) => [{ type: 'keyDown', value }, { type: 'keyUp', value }])
      await call('POST', `${sid}/actions`, { actions: [{ type: 'key', id: 'kbd', actions }] })
    },
    // Once only: tauri-driver does not answer a DELETE for a session it already closed.
    close: async () => {
      if (closed) return
      closed = true
      await call('DELETE', sid).catch(() => undefined)
    },
  }
  return d
}
