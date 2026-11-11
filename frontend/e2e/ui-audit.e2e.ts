// §17.3 UI inspection (ADR 0061): a Playwright sweep over every screen of the real desktop app — the home
// view, every registered panel, the settings dialog with each section, the command palette — at 1100 px
// and at the narrow minimum (800 px). For every interactive element (`button, a, input, select, textarea,
// [role=button|tab|menuitem|switch|checkbox|link]`) it records the bound command/event (`data-kor-cmd` /
// `data-kor-event` on it or an ancestor) and checks: an accessible name (icon-only controls labelled),
// WCAG AA contrast of its text, a visible focus indicator, no clipped text / horizontal overflow, no
// leaked translation key (`{{key}}`, `t('…')`), plus console errors per screen. Writes UI_AUDIT.md with
// screenshots under docs/ui-audit/.
//
// `npx playwright test -c e2e/playwright.config.ts --project audit` (Windows, WebView2 over CDP).
import { expect, test, chromium, type Browser, type Page } from '@playwright/test'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { until } from './driver.ts'
import { CDP_PORT, REPO, SCENARIOS, closeDesktop, launchDesktop, serve } from './harness.ts'

interface Finding { check: string; detail: string }
interface Row { screen: string; element: string; binding: string; findings: Finding[] }
interface Screen { name: string; width: number; shot: string; rows: Row[]; console: string[]; overflow: boolean; composer: number }

/** Spec §23.5: at most this many controls in the composer toolbar. */
const COMPOSER_MAX = 8

const SHOTS = join(REPO, 'docs/ui-audit')

/** Runs in the page: every interactive element of the visible UI and what is wrong with it. */
function inspect(screen: string): { rows: Row[]; overflow: boolean; composer: number } {
  const parse = (c: string): [number, number, number, number] => {
    const m = c.match(/[\d.]+/g)?.map(Number) ?? [0, 0, 0, 1]
    return [m[0] ?? 0, m[1] ?? 0, m[2] ?? 0, m[3] ?? 1]
  }
  const lum = ([r, g, b]: number[]) => {
    const f = (v: number) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4 }
    return 0.2126 * f(r!) + 0.7152 * f(g!) + 0.0722 * f(b!)
  }
  const background = (el: Element): [number, number, number] => {
    for (let e: Element | null = el; e; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor)
      if (c[3] > 0.9) return [c[0], c[1], c[2]]
    }
    return parse(getComputedStyle(document.body).backgroundColor).slice(0, 3) as [number, number, number]
  }
  const visible = (el: Element) => {
    const r = el.getBoundingClientRect()
    const s = getComputedStyle(el)
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none' && r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth
  }
  const name = (el: Element) =>
    (el.getAttribute('aria-label') ?? el.getAttribute('title') ?? (el.getAttribute('aria-labelledby') ? document.getElementById(el.getAttribute('aria-labelledby')!)?.textContent : null) ??
      (el as HTMLInputElement).labels?.[0]?.textContent ?? (el as HTMLElement).innerText ?? el.textContent ?? '').trim()
  const describe = (el: Element) => {
    const label = name(el).replace(/\s+/g, ' ').slice(0, 60)
    const tag = el.tagName.toLowerCase()
    const cls = (el.getAttribute('class') ?? '').split(/\s+/).filter(Boolean).slice(0, 2).join('.')
    return `${tag}${cls ? `.${cls}` : ''}${label ? ` "${label}"` : ''}`
  }
  const sel = 'button, a[href], input:not([type=hidden]), select, textarea, [role=button], [role=tab], [role=menuitem], [role=switch], [role=checkbox], [role=link]'
  const rows: Row[] = []
  for (const el of Array.from(document.querySelectorAll(sel)).filter(visible)) {
    const findings: Finding[] = []
    const accessible = name(el)
    if (!accessible) findings.push({ check: 'label', detail: 'no accessible name (aria-label / title / text)' })
    const hasSvgOnly = el.querySelector('svg') !== null && !((el as HTMLElement).innerText ?? '').trim()
    if (hasSvgOnly && !(el.getAttribute('aria-label') && el.getAttribute('title'))) findings.push({ check: 'icon', detail: 'icon-only control needs aria-label and title' })
    const text = ((el as HTMLElement).innerText ?? '').trim()
    if (text) {
      // The element that holds the first text (a badge inside a button has its own colours).
      const tw = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.textContent ?? '').trim() ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP })
      const holder = tw.nextNode()?.parentElement ?? el
      const s = getComputedStyle(holder)
      const fg = parse(s.color)
      const bg = background(holder)
      const [l1, l2] = [lum(fg), lum(bg)].sort((a, b) => b - a) as [number, number]
      const ratio = (l1 + 0.05) / (l2 + 0.05)
      const large = parseFloat(s.fontSize) >= 24 || (parseFloat(s.fontSize) >= 18.66 && Number(s.fontWeight) >= 700)
      const disabled = (el as HTMLButtonElement).disabled || el.getAttribute('aria-disabled') === 'true'
      if (!disabled && ratio < (large ? 3 : 4.5)) findings.push({ check: 'contrast', detail: `${ratio.toFixed(2)}:1 < ${large ? 3 : 4.5}:1 (${s.color} on rgb(${bg.join(', ')}))` })
      const he = el as HTMLElement
      if (he.scrollWidth > he.clientWidth + 1 && ['hidden', 'clip'].includes(s.overflowX) && s.textOverflow !== 'ellipsis') findings.push({ check: 'clipped', detail: `text cut at ${he.clientWidth}px (needs ${he.scrollWidth}px)` })
      if (/\{\{[\w.-]+\}\}|\bt\(['"]/.test(text)) findings.push({ check: 'i18n', detail: `translation key leaked: ${text.slice(0, 40)}` })
    }
    // Focus ring: focusing must change outline or box-shadow.
    const he = el as HTMLElement
    if (typeof he.focus === 'function' && !(el as HTMLButtonElement).disabled) {
      // A control inside a focus host (the composer card) shows focus on the host.
      const host = el.closest('[data-kor-focus-host]') ?? el
      const ring = (x: Element) => { const s = getComputedStyle(x); return `${s.outlineStyle}|${s.outlineWidth}|${s.boxShadow}|${s.borderColor}` }
      // Computed styles are read right after focus(): a running transition would still show the old value.
      // (CSSOM, not a <style> tag: the app's CSP blocks inline style sheets.)
      const still = [he, host as HTMLElement].map((x) => [x, x.style.transition] as const)
      for (const [x] of still) x.style.transition = 'none'
      // An already focused control (the palette's search box) would look the same before and after.
      if (document.activeElement === el) he.blur()
      const b = ring(el) + ring(host)
      he.focus({ preventScroll: true })
      // `:focus-visible` follows keyboard focus; `focus()` after a key press counts as keyboard.
      const a = ring(el) + ring(host)
      if (document.activeElement === el && a === b) findings.push({ check: 'focus', detail: 'no visible focus indicator' })
      he.blur()
      for (const [x, v] of still) x.style.transition = v
    }
    const bound = el.closest('[data-kor-cmd], [data-kor-event]')
    const binding = bound ? [bound.getAttribute('data-kor-cmd'), bound.getAttribute('data-kor-event')].filter(Boolean).join(' ') : ''
    rows.push({ screen, element: describe(el), binding, findings })
  }
  const bar = document.querySelector('.kor-composer-bar')
  const composer = bar ? Array.from(bar.querySelectorAll(sel)).filter(visible).length : 0
  return { rows, overflow: document.documentElement.scrollWidth > innerWidth + 1, composer }
}

function panels(): Array<{ id: string; label: string }> {
  const dir = join(REPO, 'frontend/src/desktop/renderer/panels')
  const reg = readFileSync(join(dir, 'registry.ts'), 'utf8')
  return [...reg.matchAll(/from '\.\/([\w-]+)\/index\.tsx'/g)].map((m) => {
    const src = readFileSync(join(dir, m[1]!, 'index.tsx'), 'utf8')
    return { id: m[1]!, label: /label:\s*'([^']+)'/.exec(src)?.[1] ?? m[1]! }
  })
}

function settingsSections(): string[] {
  const src = readFileSync(join(REPO, 'frontend/src/desktop/renderer/SettingsDialog.tsx'), 'utf8')
  return [...src.matchAll(/\{\s*id:\s*'[\w-]+',\s*label:\s*'([^']+)'/g)].map((m) => m[1]!)
}

test('UI audit sweep → UI_AUDIT.md', async () => {
  test.skip(process.platform !== 'win32', 'the sweep attaches to WebView2 (Windows)')
  // ~120 screens × inspect: well past the default 10 minutes.
  test.setTimeout(60 * 60_000)
  mkdirSync(SHOTS, { recursive: true })
  const served = await serve(join(SCENARIOS, 'task-approve-write.yaml'))
  const app = launchDesktop(served)
  let browser: Browser | undefined
  const screens: Screen[] = []
  try {
    await until(async () => { browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`); return true }, 60_000, 'WebView2 debugging port')
    let page: Page | undefined
    // The tray popup and the PiP status window load the same index.html: pick the window that has the composer.
    await until(async () => {
      for (const candidate of browser!.contexts().flatMap((c) => c.pages()).filter((p) => /index\.html|tauri\.localhost/.test(p.url()))) {
        const main = await candidate.evaluate(() => !document.documentElement.classList.contains('ksh-pip-root') && document.querySelector('textarea.composer-input') !== null).catch(() => false)
        if (main) {
          page = candidate
          return true
        }
      }
      return false
    }, 90_000, 'main window')
    const p = page!
    const consoleErrors: string[] = []
    p.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)) })
    p.on('pageerror', (e) => consoleErrors.push(String(e).slice(0, 200)))
    await p.locator('textarea.composer-input').waitFor({ timeout: 90_000 })
    await p.evaluate(async (dir: string) => { await (window as unknown as { kor: { setWorkspacePath(d: string): Promise<unknown> } }).kor.setWorkspacePath(dir) }, served.project)
    await p.reload()
    await p.locator('textarea.composer-input').waitFor({ timeout: 60_000 })

    const capture = async (name: string, width: number) => {
      await p.setViewportSize({ width, height: 820 })
      await p.waitForTimeout(500)
      // Last input from the keyboard, so `focus()` in the sweep shows `:focus-visible` as Tab would.
      await p.keyboard.press('Shift')
      const file = join(SHOTS, `${name.replace(/[^\w-]+/g, '-')}-${width}.png`)
      await p.screenshot({ path: file })
      const before = consoleErrors.length
      const r = await p.evaluate(inspect, name)
      screens.push({ name, width, shot: relative(REPO, file).split('\\').join('/'), rows: r.rows, overflow: r.overflow, composer: r.composer, console: consoleErrors.slice(before) })
    }
    const palette = async (label: string) => {
      await p.keyboard.press('Escape')
      await p.keyboard.press('Control+k')
      await p.keyboard.type(label)
      await p.keyboard.press('Enter')
      await p.waitForTimeout(900)
    }
    for (const width of [1100, 800]) {
      await p.keyboard.press('Escape')
      await capture('Ana ekran', width)
      await p.keyboard.press('Control+k')
      await p.waitForTimeout(400)
      await capture('Komut paleti', width)
      for (const panel of panels()) {
        await palette(panel.label)
        await capture(`Panel: ${panel.label}`, width)
      }
      await palette('Ayarlar')
      for (const section of settingsSections()) {
        const tab = p.getByRole('tab', { name: section }).or(p.getByRole('button', { name: section })).first()
        if (await tab.count()) {
          if (!(await tab.click({ timeout: 5_000 }).then(() => true, () => false))) continue
          await p.waitForTimeout(500)
          await capture(`Ayarlar: ${section}`, width)
        }
      }
      await p.keyboard.press('Escape')
    }
  } finally {
    await browser?.close().catch(() => undefined)
    closeDesktop(app)
    await served.stop().catch(() => undefined)
    // Written even when the sweep stops early, so a partial run still shows what it saw.
    writeFileSync(join(REPO, 'UI_AUDIT.md'), report(screens))
  }
  // Release gate (§17.3, §23.5): zero open findings, no sideways scroll, no console errors, a lean composer bar.
  const open = screens.flatMap((s) => s.rows.filter((r) => r.findings.length > 0).map((r) => `${s.name} (${s.width}): ${r.element}`))
  expect(screens.length, 'no screen was captured').toBeGreaterThan(0)
  expect(open, 'open UI findings (see UI_AUDIT.md)').toEqual([])
  expect(screens.filter((s) => s.overflow || s.console.length > 0 || s.composer > COMPOSER_MAX).map((s) => `${s.name} (${s.width})`)).toEqual([])
})

function report(screens: Screen[]): string {
  const all = screens.flatMap((s) => s.rows.map((r) => ({ ...r, width: s.width, shot: s.shot })))
  const open = all.filter((r) => r.findings.length > 0)
  const byCheck = new Map<string, number>()
  for (const r of open) for (const f of r.findings) byCheck.set(f.check, (byCheck.get(f.check) ?? 0) + 1)
  const lines = [
    '# UI_AUDIT — arayüz teftişi (spec §17.3)',
    '',
    `Otomatik tarama: \`frontend/e2e/ui-audit.e2e.ts\` (Playwright → WebView2, ADR 0061), ${new Date().toISOString().slice(0, 10)}.`,
    `${screens.length} ekran (1100 px ve 800 px), ${all.length} etkileşimli öğe, ${open.length} öğede bulgu.`,
    '',
    '| Denetim | Bulgu |',
    '| --- | --- |',
    ...['label', 'icon', 'contrast', 'focus', 'clipped', 'i18n'].map((c) => `| ${c} | ${byCheck.get(c) ?? 0} |`),
    `| yatay taşma (ekran) | ${screens.filter((s) => s.overflow).length} |`,
    `| composer araç çubuğunda > ${COMPOSER_MAX} kontrol (ekran) | ${screens.filter((s) => s.composer > COMPOSER_MAX).length} (en çok ${Math.max(0, ...screens.map((s) => s.composer))}) |`,
    `| konsol hatası (ekran) | ${screens.filter((s) => s.console.length > 0).length} |`,
    '',
    '## Ekranlar',
    '',
    '| Ekran | Genişlik | Öğe | Bulgulu öğe | Taşma | Konsol | Görüntü |',
    '| --- | --- | --- | --- | --- | --- | --- |',
    ...screens.map((s) => `| ${s.name} | ${s.width} | ${s.rows.length} | ${s.rows.filter((r) => r.findings.length).length} | ${s.overflow ? 'var' : '—'} | ${s.console.length || '—'} | [png](${s.shot}) |`),
    '',
    '## Öğeler (öğe → bağlı komut/olay → beklenen → gerçek → görüntü → durum)',
    '',
    '| Ekran | Öğe | Komut / olay | Beklenen | Gerçek | Görüntü | Durum |',
    '| --- | --- | --- | --- | --- | --- | --- |',
    ...all.map((r) => `| ${r.screen} (${r.width}) | ${r.element.replace(/\|/g, '\\|')} | ${r.binding || '—'} | erişilebilir ad, AA kontrast, odak halkası, kesilmeyen metin | ${r.findings.length ? r.findings.map((f) => `${f.check}: ${f.detail}`).join('; ').replace(/\|/g, '\\|') : 'uygun'} | [png](${r.shot}) | ${r.findings.length ? 'açık' : 'tamam'} |`),
    '',
    ...screens.filter((s) => s.console.length).flatMap((s) => [`### Konsol: ${s.name} (${s.width})`, '', ...s.console.map((c) => `- \`${c.replace(/`/g, "'")}\``), '']),
  ]
  return lines.join('\n') + '\n'
}
