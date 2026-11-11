// [g26] İzin yargıcı. decide() reddini gevşetmez; yalnız allow→ask/deny ve ask→deny sıkılaştırır.
import { decodedPayloads } from './exec-policy-g26.ts'
import { dangerousCommandReason } from './permissions.ts'

export type Verdict = 'pass' | 'ask' | 'deny'
export type Gate = 'allow' | 'ask' | 'deny'

/** Model "allow" dese bile çağıran bunu allow'a çevirmez. */
export function parseJudgeReply(text: string): Verdict {
  const word = text.trim().toLowerCase().split(/\s+/)[0] ?? ''
  if (word === 'deny' || word === 'reddet') return 'deny'
  if (word === 'ask' || word === 'sor') return 'ask'
  return 'pass'
}

export function tightenDecision(decision: Gate, verdict: Verdict): Gate {
  if (decision === 'deny' || verdict === 'deny') return 'deny'
  if (decision === 'ask' || verdict === 'ask') return 'ask'
  return 'allow'
}

/** Çözülmüş PowerShell gövdesi tehlikeliyse reddeder. Dışarıdan gelen metin komut üretmek için kullanılmaz. */
export function adversaryVerdict(command: string): Verdict {
  for (const payload of decodedPayloads(command)) {
    if (dangerousCommandReason(payload)) return 'deny'
  }
  if (/(?:^|\s)-ExecutionPolicy\s+Bypass\b/i.test(command)) return 'ask'
  if (/\bStart-Process\b[\s\S]{0,180}-Verb\s+RunAs\b/i.test(command)) return 'ask'
  return 'pass'
}
