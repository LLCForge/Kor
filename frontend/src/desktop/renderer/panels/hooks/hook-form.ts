// §28.12 hook editörü form durumu (ADR 0063): eylem türü (komut / MCP çağrısı / bildirim), koşullar
// (dosya deseni, dal, model) ve Claude Code dosyalarının yalnızca komut kabul etmesi.
import type { HookFile, HookItem } from '../../../ipc-ek14.ts'

export type Action = 'command' | 'mcp' | 'notify'

export interface HookForm {
  event: string
  action: Action
  command: string
  mcp: string
  args: string
  notify: string
  matcher: string
  files: string
  branch: string
  model: string
  timeout: string
  disabled: boolean
}

export const emptyForm = (event: string): HookForm => ({
  event, action: 'command', command: '', mcp: '', args: '', notify: '', matcher: '', files: '', branch: '', model: '', timeout: '', disabled: false,
})

export function formOf(it: HookItem): HookForm {
  return {
    event: it.event,
    action: it.mcp ? 'mcp' : it.notify ? 'notify' : 'command',
    command: it.command ?? '',
    mcp: it.mcp ?? '',
    args: it.args ?? '',
    notify: it.notify ?? '',
    matcher: it.matcher ?? '',
    files: it.files.join(', '),
    branch: it.branch ?? '',
    model: it.model ?? '',
    timeout: it.timeout != null ? String(it.timeout) : '',
    disabled: it.disabled,
  }
}

const opt = (s: string): string | null => (s.trim() ? s.trim() : null)

/** Formdan kaydedilecek hook; hata metni ya da öğe. */
export function itemOf(f: HookForm, file: HookFile): HookItem | string {
  if (!f.event) return 'Bir olay seçin.'
  if (file.format === 'claude' && (f.action !== 'command' || f.files.trim() || f.branch.trim() || f.model.trim() || f.disabled)) {
    return 'Claude Code dosyalarında yalnızca komut hook\'u olur; koşul, MCP ve bildirim için hooks.toml seçin.'
  }
  const timeout = f.timeout.trim() ? Number(f.timeout) : null
  if (timeout !== null && (!Number.isInteger(timeout) || timeout < 1 || timeout > 600)) return 'Zaman aşımı 1-600 saniye olmalı.'
  if (f.action === 'command' && !f.command.trim()) return 'Komut boş olamaz.'
  if (f.action === 'mcp' && !/^[^/\s]+\/[^/\s]+$/.test(f.mcp.trim())) return 'MCP çağrısı `sunucu/araç` biçiminde olmalı.'
  if (f.action === 'mcp' && f.args.trim()) {
    try {
      const v: unknown = JSON.parse(f.args)
      if (v === null || typeof v !== 'object' || Array.isArray(v)) return 'Argümanlar bir JSON nesnesi olmalı.'
    } catch {
      return 'Argümanlar geçerli JSON değil.'
    }
  }
  if (f.action === 'notify' && !f.notify.trim()) return 'Bildirim metni boş olamaz.'
  return {
    id: '',
    file: file.path,
    format: file.format,
    scope: file.scope,
    event: f.event,
    matcher: opt(f.matcher),
    command: f.action === 'command' ? f.command.trim() : null,
    timeout,
    files: f.files.split(',').map((s) => s.trim()).filter(Boolean),
    branch: opt(f.branch),
    model: opt(f.model),
    mcp: f.action === 'mcp' ? f.mcp.trim() : null,
    args: f.action === 'mcp' ? opt(f.args) : null,
    notify: f.action === 'notify' ? f.notify.trim() : null,
    disabled: f.disabled,
  }
}

/** Bir hook'un tek satırlık özeti. */
export function summary(it: HookItem): string {
  const what = it.mcp ? `MCP ${it.mcp}` : it.notify ? `Bildirim: ${it.notify}` : (it.command ?? '')
  const cond = [it.matcher && `araç ${it.matcher}`, it.files.length > 0 && `dosya ${it.files.join(',')}`, it.branch && `dal ${it.branch}`, it.model && `model ${it.model}`].filter(Boolean)
  return cond.length ? `${what} · ${cond.join(' · ')}` : what
}
