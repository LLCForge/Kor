// §21.3 "release build'de devtools sembolü yok": scans a built `kor-desktop` binary for code that only
// exists when WebView devtools are compiled in. Tauri compiles its devtools code under
// `cfg(any(debug_assertions, feature = "devtools"))`; Kor never enables the `devtools` feature
// (src-tauri/src/webview_lock.rs `devtools_feature_not_compiled`), so a release binary must have none.
//
//   node scripts/devtools-scan.ts <path to kor-desktop[.exe]>
//
// Exit 0: no marker; 1: markers found (listed); 2: usage / unreadable file.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

export interface Marker {
  text: string
  why: string
}

// Not markers: `internal_toggle_devtools` / `plugin:webview|internal_toggle_devtools` are also in the
// resolved ACL of `core:default` (embedded in every build), and WKWebView's `developerExtrasEnabled`
// key is always written (to false).
export const MARKERS: readonly Marker[] = [
  // tauri/src/webview/scripts/toggle-devtools.js, injected by the webview plugin's init script (all OS).
  { text: 'toggleDevtoolsHotkey', why: "Tauri's devtools hotkey script (Ctrl+Shift+I / Cmd+Alt+I)" },
  // wry `open_devtools` on Linux: WebKitGTK inspector import.
  { text: 'webkit_web_inspector_show', why: 'WebKitGTK inspector (wry open_devtools, Linux)' },
  // wry `open_devtools` on macOS: the private `_inspector` selector (NUL-terminated in __objc_methname).
  { text: '\0_inspector\0', why: 'WKWebView _inspector selector (wry open_devtools, macOS)' },
]

/** The markers present in `bin`. */
export function scan(bin: Buffer): Marker[] {
  return MARKERS.filter((m) => bin.includes(m.text, 0, 'latin1'))
}

function main(args: string[]): number {
  if (args.length !== 1) {
    console.error('usage: node scripts/devtools-scan.ts <kor-desktop binary>')
    return 2
  }
  let bin: Buffer
  try {
    bin = readFileSync(args[0]!)
  } catch (e) {
    console.error(`devtools-scan: cannot read ${args[0]}: ${e instanceof Error ? e.message : String(e)}`)
    return 2
  }
  const found = scan(bin)
  if (found.length === 0) {
    console.log(`devtools-scan: ${args[0]}: no devtools code (${bin.length} bytes, ${MARKERS.length} markers)`)
    return 0
  }
  console.error(`devtools-scan: ${args[0]} contains devtools code (§21.1 forbids it in release builds):`)
  for (const m of found) console.error(`  ${JSON.stringify(m.text)}: ${m.why}`)
  return 1
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  process.exitCode = main(process.argv.slice(2))
}
