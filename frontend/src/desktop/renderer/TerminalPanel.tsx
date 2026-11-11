// Entegre terminal paneli (cdesktop Terminal görünümü). Tek-atım komut çalıştırır
// (gerçek PTY değil); çoğu komut için kullanışlı, etkileşimli TUI'ları desteklemez.
import { CornerDownLeft } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api, errorMessage } from './api.ts'

interface Entry {
  command: string
  output: string
  code: number | null
}

export function TerminalPanel() {
  const [entries, setEntries] = useState<Entry[]>([])
  const [command, setCommand] = useState('')
  const [busy, setBusy] = useState(false)
  const [history, setHistory] = useState<string[]>([])
  const [histIdx, setHistIdx] = useState<number | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight })
  }, [entries, busy])

  const run = async () => {
    const cmd = command.trim()
    if (!cmd || busy) return
    setBusy(true)
    setCommand('')
    setHistory((h) => [...h, cmd])
    setHistIdx(null)
    try {
      const res = await api.terminal.run(cmd)
      setEntries((e) => [...e, { command: cmd, output: res.output, code: res.code }])
    } catch (err) {
      setEntries((e) => [...e, { command: cmd, output: errorMessage(err), code: null }])
    } finally {
      setBusy(false)
    }
  }

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      void run()
    } else if (e.key === 'ArrowUp' && history.length > 0) {
      e.preventDefault()
      const i = histIdx === null ? history.length - 1 : Math.max(0, histIdx - 1)
      setHistIdx(i)
      setCommand(history[i] ?? '')
    } else if (e.key === 'ArrowDown' && histIdx !== null) {
      e.preventDefault()
      const i = histIdx + 1
      if (i >= history.length) {
        setHistIdx(null)
        setCommand('')
      } else {
        setHistIdx(i)
        setCommand(history[i] ?? '')
      }
    }
  }

  return (
    <div className="terminal-panel">
      <div className="terminal-scroll" ref={scrollRef}>
        {entries.length === 0 && !busy && (
          <p className="terminal-hint">Komut yaz ve Enter'a bas. Çalışma klasöründe tek-atım olarak çalışır.</p>
        )}
        {entries.map((e, i) => (
          <div key={i} className="terminal-entry">
            <div className="terminal-cmd">
              <span className="terminal-prompt">$</span> {e.command}
              {e.code !== null && e.code !== 0 && <span className="terminal-code">exit {e.code}</span>}
            </div>
            {e.output && <pre className="terminal-out">{e.output}</pre>}
          </div>
        ))}
        {busy && <div className="terminal-busy">çalışıyor…</div>}
      </div>
      <div className="terminal-input-row">
        <span className="terminal-prompt">$</span>
        <input
          className="terminal-input"
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          onKeyDown={onKey}
          placeholder="komut…"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          disabled={busy}
          aria-label="Terminal komutu"
        />
        <button type="button" className="action-btn" onClick={() => void run()} disabled={busy} aria-label="Çalıştır" title="Çalıştır (Enter)">
          <CornerDownLeft size={15} aria-hidden="true" />
        </button>
      </div>
    </div>
  )
}
