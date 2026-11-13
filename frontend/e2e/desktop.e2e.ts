// §16.4 / §17.2 on the desktop surface (ADR 0061): every scenario in tests/e2e/scenarios against the real
// app and core with the mock provider, checked by the same Rust checker as the CLI and TUI; then every
// registered panel's screenshot (visual regression) in dark and light with reduced motion.
//
// Windows: Playwright attaches to WebView2 (CDP). Elsewhere `KOR_E2E_DRIVER=webdriver` drives the app
// through `tauri-driver` (start it first: `tauri-driver --port 4444`); screenshots are Windows-only.
import { test, expect } from '@playwright/test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { USE_WEBDRIVER, attach } from './app.ts'
import { SCENARIOS, check, serve } from './harness.ts'
import { costPanelText, openProject, runSteps } from './flow.ts'

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
      // §32.2: no unnecessary re-renders while the scenario streamed (debug build: perfmon/rerenders.ts).
      const rerenders = await d.run(() => {
        const f = (window as unknown as { __korRerenders?: () => { total: number; byComponent: Record<string, number> } }).__korRerenders
        return f ? f() : null
      })
      expect(rerenders, 'the renderer is a development build (rerender tracking on)').not.toBeNull()
      expect(rerenders!.total, JSON.stringify(rerenders!.byComponent)).toBe(0)
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
