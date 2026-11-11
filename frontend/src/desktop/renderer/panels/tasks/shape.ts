// Görevler paneli için saf yardımcılar (node --test ile sınanır).
import type { SubStatus, SubTask, TaskState, TaskStatus } from '../../../ipc-tasks.ts'

export const TASK_STATUS: Record<TaskStatus, string> = {
  planning: 'Planlanıyor',
  running: 'Çalışıyor',
  merging: 'Birleştiriliyor',
  conflict: 'Çakışma',
  done: 'Tamamlandı',
  failed: 'Yarım kaldı',
  cancelled: 'İptal edildi',
}

export const SUB_STATUS: Record<SubStatus, string> = {
  pending: 'Bekliyor',
  running: 'Çalışıyor',
  approved: 'Onaylandı',
  rejected: 'Reddedildi',
  failed: 'Başarısız',
  merged: 'Birleştirildi',
  conflict: 'Çakışma',
  dropped: 'Bırakıldı',
  cancelled: 'İptal edildi',
}

/** Görevin sürüp sürmediği (iptal düğmesi, devam düğmesi). */
export function isActive(s: TaskStatus): boolean {
  return s === 'planning' || s === 'running' || s === 'merging'
}

export function usd(v: number, estimated: boolean): string {
  const text = v > 0 && v < 0.01 ? '<$0.01' : `$${v.toFixed(2)}`
  return estimated ? `≈${text}` : text
}

export function totalUsd(t: TaskState): number {
  return t.orchestrator_usd + t.reviewer_usd + t.subtasks.reduce((a, s) => a + s.cost_usd, 0)
}

export function anyEstimated(t: TaskState): boolean {
  return t.estimated || t.subtasks.some((s) => s.estimated)
}

/** Birleştirilmiş ya da bırakılmış alt görevlerin yüzdesi. */
export function percent(t: TaskState): number {
  if (t.status === 'done') return 100
  if (!t.subtasks.length) return 0
  const done = t.subtasks.filter((s) => s.status === 'merged' || s.status === 'dropped').length
  return Math.floor((done * 100) / t.subtasks.length)
}

export function modelText(s: SubTask): string {
  if (s.model) return s.model
  if (s.models_used.length) return s.models_used.join(', ')
  return 'Yönlendirici seçer'
}

/** Gelen denetim noktasını listeye yerleştirir (yeni görev başa, en son güncellenen önce). */
export function upsert(list: readonly TaskState[], t: TaskState): TaskState[] {
  const rest = list.filter((x) => x.id !== t.id)
  return [t, ...rest].sort((a, b) => b.updated_ms - a.updated_ms)
}

/** `backend=openai/gpt-4.1, docs=local/qwen3` → { backend: …, docs: … }; hatalı parçalar `bad` içinde. */
export function parseModels(text: string): { models: Record<string, string>; bad: string[] } {
  const models: Record<string, string> = {}
  const bad: string[] = []
  for (const raw of text.split(/[,\n]/)) {
    const part = raw.trim()
    if (!part) continue
    const i = part.indexOf('=')
    const role = i > 0 ? part.slice(0, i).trim() : ''
    const model = i > 0 ? part.slice(i + 1).trim() : ''
    if (!role || !/^[\w.-]+\/\S+$/.test(model)) bad.push(part)
    else models[role] = model
  }
  return { models, bad }
}
