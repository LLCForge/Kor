// [g26] Komut satırını kabuk operatörlerinden ve PowerShell -EncodedCommand gövdesinden ayırır.
// Çözülen metin tehlikeli-komut denetimine gider; yeni bir komut üretilmez.

const POWERSHELL = /^(powershell|pwsh)$/i
const MAX_DECODED = 8_000

/** Tırnak dışındaki ; | && || ve tek & ile böler. */
export function splitSegments(command: string): string[] {
  const parts: string[] = []
  let current = ''
  let quote: '"' | "'" | null = null
  for (let i = 0; i < command.length; i++) {
    const ch = command[i]
    if (quote) {
      current += ch
      if (ch === quote) quote = null
      continue
    }
    if (ch === '"' || ch === "'") {
      quote = ch
      current += ch
      continue
    }
    const two = command.slice(i, i + 2)
    if (two === '&&' || two === '||') {
      if (current.trim()) parts.push(current.trim())
      current = ''
      i += 1
      continue
    }
    if (ch === ';' || ch === '|' || ch === '&' || ch === '\n') {
      if (current.trim()) parts.push(current.trim())
      current = ''
      continue
    }
    current += ch
  }
  if (current.trim()) parts.push(current.trim())
  return parts
}

function firstToken(segment: string): string {
  const raw = segment.trim().split(/\s+/)[0] ?? ''
  return raw.replace(/^["']|["']$/g, '').replace(/^.*[\\/]/, '').replace(/\.(exe|cmd|bat|com)$/i, '')
}

/** UTF-16LE base64. Bozuk veya aşırı uzun gövde yok sayılır. */
export function decodePowerShellEncoded(payload: string): string | null {
  const cleaned = payload.trim()
  if (cleaned.length < 8 || cleaned.length > 20_000 || !/^[A-Za-z0-9+/=]+$/.test(cleaned)) return null
  try {
    const text = Buffer.from(cleaned, 'base64').toString('utf16le')
    if (!text.trim() || text.length > MAX_DECODED) return null
    return text
  } catch {
    return null
  }
}

function encodedFlag(segment: string): string | null {
  const match = /(?:^|\s)-(?:EncodedCommand|encodedcommand|enc|ec|e)\s+([A-Za-z0-9+/=]+)/i.exec(segment)
  return match?.[1] ?? null
}

function commandFlag(segment: string): string | null {
  const match = /(?:^|\s)-(?:Command|c)\s+([\s\S]+)/i.exec(segment)
  return match?.[1]?.trim() || null
}

/** Yalnızca powershell/pwsh parçalarının çözülmüş gövdeleri. Özyineleme iki düzeyde durur. */
export function decodedPayloads(command: string, depth = 0): string[] {
  if (depth > 2) return []
  const out: string[] = []
  for (const segment of splitSegments(command)) {
    const exe = firstToken(segment)
    if (!POWERSHELL.test(exe)) continue
    const encoded = encodedFlag(segment)
    if (encoded) {
      const text = decodePowerShellEncoded(encoded)
      if (text) {
        out.push(text)
        out.push(...decodedPayloads(text, depth + 1))
      }
    }
    const nested = commandFlag(segment)
    if (nested) {
      out.push(nested)
      out.push(...decodedPayloads(nested, depth + 1))
    }
  }
  return out
}

/** Yönlendirme hedefleri (tırnak dışı). */
export function redirectTargets(command: string): string[] {
  const targets: string[] = []
  let quote: '"' | "'" | null = null
  for (let i = 0; i < command.length; i++) {
    const ch = command[i]
    if (quote) {
      if (ch === quote) quote = null
      continue
    }
    if (ch === '"' || ch === "'") {
      quote = ch
      continue
    }
    if (ch !== '>') continue
    let j = i + 1
    if (command[j] === '>') j += 1
    while (command[j] === ' ') j += 1
    let target = ''
    const q = command[j]
    if (q === '"' || q === "'") {
      j += 1
      while (j < command.length && command[j] !== q) target += command[j++]
    } else {
      while (j < command.length && command[j] !== ' ' && command[j] !== ';' && command[j] !== '|' && command[j] !== '&') target += command[j++]
    }
    if (target) targets.push(target)
    i = j
  }
  return targets
}
