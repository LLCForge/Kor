// A scenario on the desktop surface (§16.4 "Uçtan uca", §17.2), the same steps the CLI and TUI take:
// open the project, type the prompt in the composer, answer the approval card with its buttons, run
// `command` steps through the parity desktop method, then read the cost panel.
import type { Driver } from './driver.ts'
import { until } from './driver.ts'
import { desktopMethod, type Served } from './harness.ts'

const COMPOSER = 'textarea.composer-input'

// Functions passed to `d.run` are serialized into the page: they may only use what is there (`window.kor`).
type Kor = Record<string, unknown> & {
  setWorkspacePath(p: string): Promise<unknown>
  liveSessions(): Promise<unknown[]>
}

/** Opens the scenario's project in the app and waits for the composer. */
export async function openProject(d: Driver, project: string): Promise<void> {
  await d.waitFor(COMPOSER, 90_000)
  await d.run(async (p: string) => { await (window as unknown as { kor: Kor }).kor.setWorkspacePath(p) }, project)
  // Same permission mode as the CLI/TUI runs (`default`: edits ask), so the approval card appears.
  await d.run(async () => {
    const k = (window as unknown as { kor: { setPermissionMode(m: string): Promise<void>; setDefaultPermissionMode(m: string): Promise<void> } }).kor
    await k.setDefaultPermissionMode('default')
    await k.setPermissionMode('default')
  })
  // No reload: the WebView lock (§21) treats a reload as leaving the app. The next turn runs in this root.
  await d.waitFor(COMPOSER, 60_000)
}

/** The turn is over: nothing live in the main process for a moment (two quiet polls). */
async function turnEnd(d: Driver): Promise<void> {
  let quiet = 0
  await until(async () => {
    const live = await d.run(async () => (await (window as unknown as { kor: Kor }).kor.liveSessions()).length)
    quiet = live === 0 ? quiet + 1 : 0
    return quiet >= 2
  }, 120_000, 'the turn to end')
}

export async function runSteps(d: Driver, served: Served): Promise<string> {
  let output = ''
  const steps = served.scenario.steps
  for (let i = 0; i < steps.length; i++) {
    const st = steps[i]!
    const nextIsApproval = steps[i + 1]?.approve !== undefined
    if (st.prompt !== undefined) {
      await d.fill(COMPOSER, st.prompt)
      await d.press(COMPOSER, 'Enter')
      if (nextIsApproval) await d.waitFor('.approval button.approve', 120_000)
      else {
        await new Promise((r) => setTimeout(r, 400))
        await turnEnd(d)
      }
    } else if (st.approve !== undefined) {
      const button = st.approve === 'no' ? '.approval button.deny' : '.approval button.approve'
      // "always": the review's second primary button (`Bu oturumda hep izin ver`) answers `always`.
      await d.click(st.approve === 'always' ? '.approval .approval-actions button.btn:not(.approve):not(.deny)' : button)
      await turnEnd(d)
    } else if (st.command !== undefined) {
      const method = desktopMethod(st.command)
      const result = await d.run(async (m: string, args: unknown[]) => {
        const [ns, name] = m.includes('.') ? m.split('.') as [string, string] : ['', m]
        const bridge = (window as unknown as { kor: Kor }).kor as unknown as Record<string, Record<string, (...a: unknown[]) => Promise<unknown>>> & Record<string, (...a: unknown[]) => Promise<unknown>>
        let a = args
        // `core.undo(session)`: the latest undo journal's session, as the Undo button picks it.
        if (m === 'core.undo' && a.length === 0) {
          const list = (await bridge.core!.undoList!()) as Array<{ session: string }>
          a = [list[0]?.session ?? '']
        }
        const r = ns ? await bridge[ns]![name]!(...a) : await (bridge[name] as unknown as (...x: unknown[]) => Promise<unknown>)(...a)
        return JSON.stringify(r)
      }, method, st.desktop_args ?? [])
      output += `${result}\n`
      // A task runs in the background (not a chat session): wait until its status is final.
      if (method === 'tasks.start') {
        const id = (JSON.parse(result) as { id: string }).id
        await until(async () => {
          const st = await d.run(async (i: string) => {
            const k = (window as unknown as { kor: { tasks: { status(id: string): Promise<{ status: string }> } } }).kor
            return (await k.tasks.status(i)).status
          }, id)
          return ['done', 'failed', 'cancelled', 'conflict'].includes(st)
        }, 300_000, `task #${id} to finish`)
        await turnEnd(d)
      }
    }
  }
  return output
}

/** Cost dashboard after the run (§16.4: "maliyet panosu güncellenir"): the panel's text. */
export async function costPanelText(d: Driver): Promise<string> {
  // Through the command palette (Ctrl+K → the panel's name), as a user opens it.
  await d.keys('Control+k')
  await d.waitFor('[role="dialog"] input', 10_000)
  await d.type('Maliyet')
  await d.keys('Enter')
  try {
    await d.waitFor('[data-kor-cmd~="cost.summary"]', 15_000)
    await new Promise((r) => setTimeout(r, 1500))
    return await d.text('[data-kor-cmd~="cost.summary"]')
  } catch {
    // The palette entry did not open the panel: read what the panel reads (`cost.summary`), and say so.
    await d.keys('Escape')
    const r = await d.run(async () => JSON.stringify(await (window as unknown as { kor: { cost: { summary(days: number): Promise<unknown> } } }).kor.cost.summary(30)))
    return `[cost panel not reached through the palette] ${r}`
  }
}
