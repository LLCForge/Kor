// [g26] Windows'ta çekirdek sanal alanı yoktur (g12 bunu bilerek çalıştırmaz).
// commandPolicy: true iken komut kullanıcı kipinde kalır: ağ, yükseltme ve çalışma dizini dışı yazma çalışmadan kesilir.
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { z } from 'zod'
import { redirectTargets } from './exec-policy-g26.ts'
import { dangerousCommandReason } from './permissions.ts'
import { settingsFiles } from './settings.ts'

const schema = z.object({ commandPolicy: z.boolean() }).partial()

export function commandPolicyEnabled(file: string = settingsFiles('.').user): boolean {
  try {
    const parsed = schema.safeParse(JSON.parse(readFileSync(file, 'utf8')))
    return parsed.success && parsed.data.commandPolicy === true
  } catch {
    return false
  }
}

const WIN_ABS = /^[A-Za-z]:[\\/]|^\\\\/
function insideRoot(root: string, target: string): boolean {
  if (!target || target.startsWith('&')) return true
  // Windows yolları (C:\…) Linux/macOS'ta da Windows kurallarıyla çözülür (CI ve testler).
  const p = WIN_ABS.test(root) || WIN_ABS.test(target) ? path.win32 : path
  const abs = p.resolve(root, target)
  const rel = p.relative(root, abs)
  return rel === '' || (!rel.startsWith('..') && !p.isAbsolute(rel))
}

/** Politika açıkken spawn'dan önce. null: komut çalışabilir. */
export function policyBlockReason(command: string, root: string): string | null {
  const danger = dangerousCommandReason(command)
  if (danger) return danger
  if (/\b(curl|wget|iwr|irm|Invoke-WebRequest|Invoke-RestMethod|ssh|scp|sftp)\b/i.test(command)) return 'politika modunda ağ komutu kapalı'
  if (/\bStart-Process\b[\s\S]{0,180}-Verb\s+RunAs\b/i.test(command) || /\bsudo\b/.test(command)) return 'politika modunda yetki yükseltme kapalı'
  for (const target of redirectTargets(command)) {
    if (!insideRoot(root, target)) return `çalışma dizini dışına yazma (${target})`
  }
  return null
}
