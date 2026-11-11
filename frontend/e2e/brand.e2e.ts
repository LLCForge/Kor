// §16.4 "Masaüstü": KorThinking animation states (glow / hot / spark), KorSpinner, KorDots, KorStatus,
// reduced motion and dark / light, from the single brand source `assets/KorThinking.tsx` +
// `assets/kor-thinking.css` (ADR 0061). Each animation is frozen at a fixed time, so screenshots are exact.
import { test, expect, type Page } from '@playwright/test'
import { build } from 'esbuild'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { FRONTEND, REPO } from './harness.ts'

let bundle = ''
const css = readFileSync(join(REPO, 'assets/kor-thinking.css'), 'utf8')

test.beforeAll(async () => {
  const out = await build({
    stdin: {
      contents: `
        import { createRoot } from 'react-dom/client'
        import { KorThinking, KorSpinner, KorDots, KorStatus } from './assets/KorThinking.tsx'
        createRoot(document.getElementById('root')).render(
          <div style={{ display: 'flex', gap: 24, alignItems: 'center', padding: 24 }}>
            <div data-part="cube"><KorThinking size={160} /></div>
            <div data-part="indicators" style={{ display: 'flex', gap: 16, alignItems: 'center' }}>
              <KorSpinner size={28} /><KorDots width={48} /><KorStatus verb="Közlüyor" elapsedMs={65000} tokens={1240} />
            </div>
          </div>)`,
      resolveDir: REPO,
      loader: 'tsx',
    },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    nodePaths: [join(FRONTEND, 'node_modules')],
    loader: { '.css': 'empty' },
    logLevel: 'silent',
  })
  bundle = out.outputFiles[0]!.text
})

async function render(page: Page, scheme: 'dark' | 'light', reducedMotion: 'reduce' | 'no-preference') {
  await page.emulateMedia({ colorScheme: scheme, reducedMotion })
  const bg = scheme === 'dark' ? '#14151B' : '#F1EBDA'
  await page.setContent(`<!doctype html><html><head><meta charset="utf-8"><style>body{margin:0;background:${bg}}${css}</style></head><body><div id="root"></div></body></html>`)
  await page.addScriptTag({ content: bundle })
  await page.locator('[data-part="cube"] svg').waitFor()
}

/** Pause every animation at `ms` (Web Animations API): one deterministic frame per state. */
async function freeze(page: Page, ms: number) {
  await page.evaluate((t) => {
    for (const a of document.getAnimations()) {
      a.pause()
      a.currentTime = t
    }
  }, ms)
}

// Keyframe times from kor-thinking.css: glow and hot share a 1.8 s cycle (peak at half), sparks run 2 s.
const STATES = [
  { name: 'glow', ms: 900 },
  { name: 'hot', ms: 450 },
  { name: 'spark', ms: 1500 },
] as const

for (const scheme of ['dark', 'light'] as const) {
  for (const s of STATES) {
    test(`KorThinking ${s.name} (${scheme})`, async ({ page }) => {
      await render(page, scheme, 'no-preference')
      expect(await page.evaluate(() => document.getAnimations().length)).toBeGreaterThan(3)
      await freeze(page, s.ms)
      await expect(page.locator('[data-part="cube"]')).toHaveScreenshot(`kor-thinking-${s.name}-${scheme}.png`, { animations: 'allow' })
    })
  }
  test(`indicators (${scheme})`, async ({ page }) => {
    await render(page, scheme, 'no-preference')
    await freeze(page, 600)
    await expect(page.locator('[data-part="indicators"]')).toHaveScreenshot(`kor-indicators-${scheme}.png`, { animations: 'allow' })
  })
  test(`reduced motion stops every animation (${scheme})`, async ({ page }) => {
    await render(page, scheme, 'reduce')
    // §16.2 rule 3: nothing moves; the meaning stays (role="img" + aria-label).
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0)
    await expect(page.getByRole('img', { name: 'Kor düşünüyor' })).toBeVisible()
    await expect(page.locator('[data-part="cube"]')).toHaveScreenshot(`kor-thinking-reduced-${scheme}.png`)
  })
}
