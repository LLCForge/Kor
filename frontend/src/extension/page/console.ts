// Sayfanın kendi dünyasında (MAIN) çalışır, yalnızca Kor grubundaki sekmelere enjekte edilir (ADR 0058):
// console çıktısını, yakalanmamış hataları ve reddedilen sözleri son 300 satırla `window.__korConsole`a yazar.
// browser_console bunu okur. Sayfa içeriği güvenilmezdir; satırlar araç çıktısında öyle işaretlenir.
interface KorConsoleWindow {
  __korConsole?: string[]
}

const w = window as unknown as KorConsoleWindow
if (!w.__korConsole) {
  const lines: string[] = []
  w.__korConsole = lines
  const push = (kind: string, args: unknown[]) => {
    const text = args
      .map((a) => {
        if (typeof a === 'string') return a
        try {
          return JSON.stringify(a)
        } catch {
          return String(a)
        }
      })
      .join(' ')
    lines.push(`[${kind}] ${text}`.slice(0, 2000))
    if (lines.length > 300) lines.splice(0, lines.length - 300)
  }
  for (const kind of ['log', 'info', 'warn', 'error', 'debug'] as const) {
    const orig = console[kind].bind(console)
    console[kind] = (...args: unknown[]) => {
      push(kind, args)
      orig(...args)
    }
  }
  addEventListener('error', (e) => push('exception', [e.message, `${e.filename}:${e.lineno}`]))
  addEventListener('unhandledrejection', (e) => push('rejection', [e.reason]))
}
