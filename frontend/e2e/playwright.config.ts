// Desktop E2E, visual regression and UI audit (spec §16.4, §17.3; ADR 0061).
// `npx playwright test -c e2e/playwright.config.ts [--project brand|desktop|audit]`
// - brand: KorThinking states, reduced motion, dark/light — bundled from assets/ into Playwright's own
//   Chromium, so it runs on all three OS (`npx playwright install chromium`).
// - desktop: the real app (Windows: WebView2 over CDP) — scenarios, every panel's screenshot.
// - audit: the §17.3 sweep that writes UI_AUDIT.md (run on demand).
import { defineConfig } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

// §17.2: the HTML report of a run is kept per release as `reports/e2e-<version>/index.html` at the repository root.
const version = (JSON.parse(readFileSync(resolve(fileURLToPath(import.meta.url), '../../package.json'), 'utf8')) as { version: string }).version

export default defineConfig({
  testDir: '.',
  timeout: 10 * 60_000,
  workers: 1,
  fullyParallel: false,
  reporter: [['list'], ['html', { open: 'never', outputFolder: `../../reports/e2e-${version}` }]],
  outputDir: '../test-results/e2e-output',
  // Baselines per OS and project: fonts and WebView rendering differ between platforms.
  snapshotPathTemplate: '{testDir}/__screenshots__/{projectName}/{platform}/{arg}{ext}',
  expect: {
    // §17.3: pixel difference threshold 0.1 %.
    toHaveScreenshot: { maxDiffPixelRatio: 0.001, animations: 'disabled', caret: 'hide' },
  },
  projects: [
    // KOR_E2E_CHANNEL=msedge uses the installed Edge instead of a downloaded Chromium.
    { name: 'brand', testMatch: /brand\.e2e\.ts$/, use: { channel: process.env.KOR_E2E_CHANNEL } },
    // macOS (§16.4): WKWebView has no automation protocol; the brand states in Playwright's WebKit are the
    // closest engine an automated run reaches there (`npx playwright install webkit`).
    { name: 'brand-webkit', testMatch: /brand\.e2e\.ts$/, use: { browserName: 'webkit' } },
    // responsive.e2e.ts: no horizontal overflow at 800×600 and 1920×1080 (same WebView2 attach).
    // webview-lock.e2e.ts: §21.3 blocked shortcuts and right-click menus on the real app.
    { name: 'desktop', testMatch: /(desktop|responsive|webview-lock)\.e2e\.ts$/ },
    { name: 'audit', testMatch: /ui-audit\.e2e\.ts$/ },
    // §16.4 fourth surface: the Chrome extension's side panel in Playwright's Chromium (`npm run build:extension`);
    // the scenarios also need the desktop app and KOR_E2E_SIDEPANEL=1 (sidepanel.e2e.ts).
    { name: 'sidepanel', testMatch: /sidepanel\.e2e\.ts$/ },
  ],
})
