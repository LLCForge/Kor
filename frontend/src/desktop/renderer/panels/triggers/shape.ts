// Tetikleyici paneli için saf yardımcılar (node --test ile sınanır).
import type { JobStatus, TriggerEvent, TriggerInput, TriggerRow } from '../../../ipc-triggers.ts'

export const EVENT_LABEL: Record<TriggerEvent, string> = {
  git_push: 'Git push',
  cron: 'Zamanlama (cron)',
  webhook: 'Webhook',
  log_match: 'Günlük eşleşmesi',
}

export const STATUS_LABEL: Record<JobStatus, string> = {
  queued: 'Kuyrukta',
  running: 'Çalışıyor',
  waiting: 'Onay bekliyor',
  done: 'Bitti',
  failed: 'Başarısız',
}

/** Boş form ya da düzenlenecek tetikleyicinin formu. */
export function formFrom(row: TriggerRow | null): TriggerInput {
  if (!row) return { name: '', event: 'git_push', task: '', enabled: true, branch: 'main', approval: 'notify', pr: false }
  return {
    name: row.name,
    event: row.event,
    task: row.task,
    enabled: row.enabled,
    branch: row.branch,
    schedule: row.schedule,
    path: row.path,
    secret: row.secret,
    source: row.source,
    pattern: row.pattern,
    debounceSecs: row.debounceSecs,
    model: row.model,
    budgetUsd: row.budgetUsd,
    approval: row.approval,
    pr: row.pr,
  }
}

const CRON_FIELD = /^(\*|\d+(-\d+)?)(\/\d+)?(,(\*|\d+(-\d+)?)(\/\d+)?)*$/

/** Kaydetmeden önce gösterilecek ilk sorun (son söz arka uçta); sorun yoksa null. */
export function formProblem(f: TriggerInput): string | null {
  if (!f.name.trim()) return 'Ad gerekli'
  if (/[/\\]/.test(f.name)) return 'Ad / ya da \\ içeremez'
  if (!f.task.trim()) return 'Görev metni gerekli'
  if (f.event === 'cron') {
    const parts = (f.schedule ?? '').trim().split(/\s+/)
    if (parts.length !== 5 || !parts.every((p) => CRON_FIELD.test(p))) return 'Zamanlama beş alan olmalı: dakika saat gün ay haftanın-günü'
  }
  if (f.event === 'webhook') {
    if (!(f.path ?? '').startsWith('/')) return 'Yol / ile başlamalı'
    if (!(f.secret ?? '').trim()) return 'Webhook için kasadaki HMAC sırrının adı gerekli'
  }
  if (f.event === 'log_match') {
    if (!(f.source ?? '').trim()) return 'Kaynak gerekli (dosya yolu, journald:birim ya da docker:kapsayıcı)'
    if (!(f.pattern ?? '').trim()) return 'Desen gerekli'
    try {
      new RegExp(f.pattern ?? '')
    } catch {
      return 'Desen geçerli bir düzenli ifade değil'
    }
  }
  if (f.budgetUsd != null && (!Number.isFinite(f.budgetUsd) || f.budgetUsd < 0)) return 'Bütçe sıfır ya da pozitif olmalı'
  return null
}

/** Olaya ait olmayan alanları atar (arka uç zaten yok sayar; dosyada iz bırakmasın). */
export function forSave(f: TriggerInput): TriggerInput {
  const base: TriggerInput = { name: f.name.trim(), event: f.event, task: f.task.trim(), enabled: f.enabled, approval: f.approval, pr: f.pr, model: f.model?.trim() || undefined, budgetUsd: f.budgetUsd || undefined }
  switch (f.event) {
    case 'git_push':
      return { ...base, branch: f.branch?.trim() ?? '', secret: f.secret?.trim() || undefined }
    case 'cron':
      return { ...base, schedule: f.schedule?.trim() }
    case 'webhook':
      return { ...base, path: f.path?.trim(), secret: f.secret?.trim() }
    case 'log_match':
      return { ...base, source: f.source?.trim(), pattern: f.pattern, debounceSecs: f.debounceSecs || undefined }
  }
}

/** `≈$0.12` / `$0.00` (tahmini tutarlar ≈ ile, §5.2). */
export function usd(v: number, estimated: boolean): string {
  return `${estimated ? '≈' : ''}$${v.toFixed(2)}`
}

/** Bir işin süresi: `1 dk 5 sn`, sürüyorsa şimdiye kadar. */
export function duration(started: number | null, finished: number | null, now: number): string {
  if (started == null) return '—'
  const secs = Math.max(0, Math.round(((finished ?? now) - started) / 1000))
  const m = Math.floor(secs / 60)
  return m ? `${m} dk ${secs % 60} sn` : `${secs} sn`
}
