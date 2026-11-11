// [g26] HAR 1.2 ve el değiştirme belgesi. Çerez ve yetki başlığı yazılmaz.

export interface HarRecord {
  started: string
  method: string
  url: string
  status: number
  type: string
}

export function redactUrl(raw: string): string {
  try {
    const url = new URL(raw)
    url.username = ''
    url.password = ''
    return url.toString().slice(0, 2000)
  } catch {
    return raw.slice(0, 2000)
  }
}

export function buildHar(entries: readonly HarRecord[]): string {
  return JSON.stringify({
    log: {
      version: '1.2',
      creator: { name: 'Kor', version: '1.0' },
      entries: entries.slice(-200).map((entry) => ({
        startedDateTime: entry.started,
        request: { method: entry.method, url: redactUrl(entry.url), headers: [] },
        response: { status: entry.status, statusText: '', headers: [], content: { mimeType: entry.type } },
      })),
    },
  })
}

export function parseSnapshotRefs(lines: readonly string[]): Array<{ ref: string; tag: string; name: string }> {
  const refs: Array<{ ref: string; tag: string; name: string }> = []
  for (const line of lines) {
    const match = /^(e\d+)\s+<([a-z0-9]+)[^>]*>\s*(.*)$/i.exec(line.trim())
    if (!match?.[1] || !match[2]) continue
    refs.push({ ref: match[1], tag: match[2].toLowerCase(), name: (match[3] ?? '').trim() })
  }
  return refs
}

export function handoffDocument(input: { url: string; title: string; text: string; refs: readonly string[] }): string {
  return JSON.stringify({
    url: redactUrl(input.url),
    title: input.title.slice(0, 300),
    text: input.text.slice(0, 8000),
    refs: input.refs.slice(0, 200),
  })
}
