// Denetim günlüğü (audit/*.jsonl) okuma — saf parçası; electron/node fs yok, test edilebilir.
import type { LogEntry } from './ipc-logs.ts'

/** JSONL metnini kayıtlara çevirir; bozuk satırlar atlanır; sessionId verilirse yalnız o sohbet. */
export function parseAuditLines(text: string, sessionId: string | null): LogEntry[] {
  const out: LogEntry[] = []
  for (const line of text.split('\n')) {
    const t = line.trim()
    if (!t) continue
    let rec: Record<string, unknown>
    try {
      rec = JSON.parse(t) as Record<string, unknown>
    } catch {
      continue
    }
    if (sessionId && rec.sessionId !== sessionId) continue
    if (typeof rec.ts !== 'string' || typeof rec.tool !== 'string') continue
    const decision = rec.decision
    if (decision !== 'allow' && decision !== 'deny' && decision !== 'approved' && decision !== 'rejected' && decision !== 'invalid') continue
    out.push({
      ts: rec.ts,
      tool: rec.tool,
      decision,
      ...(typeof rec.ok === 'boolean' ? { ok: rec.ok } : {}),
      ...(typeof rec.durationMs === 'number' ? { durationMs: rec.durationMs } : {}),
      ...(typeof rec.reason === 'string' ? { reason: rec.reason } : {}),
      args: typeof rec.args === 'string' ? rec.args : JSON.stringify(rec.args ?? {}),
    })
  }
  return out
}

/** Günlük aramasında eşleşme: araç adı, karar, gerekçe ve argüman metni. */
export function matchesLogQuery(entry: LogEntry, query: string): boolean {
  const q = query.trim().toLocaleLowerCase('tr')
  if (!q) return true
  return `${entry.tool} ${entry.decision} ${entry.reason ?? ''} ${entry.args}`.toLocaleLowerCase('tr').includes(q)
}
