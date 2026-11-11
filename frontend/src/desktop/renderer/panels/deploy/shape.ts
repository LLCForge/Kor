// Dağıtım günlüğü durumu: `kor:deploy/event` olaylarını adım satırlarına çevirir (saf, test edilir).
import type { DeployEvent, DeployPlan, DeployStep, Risk } from '../../../ipc-deploy.ts'

export type RowState = 'pending' | 'running' | 'ok' | 'failed' | 'skipped'

export interface StepRow {
  step: DeployStep
  state: RowState
  output: string[]
  detail: string
}

const MAX_OUTPUT = 200

export function rowsOf(plan: DeployPlan): StepRow[] {
  return plan.steps.map((step) => ({ step, state: 'pending', output: [], detail: '' }))
}

/** Bir olayı uygular. Plan dışı adımlar (geri alma, `docker inspect`) sona eklenir. */
export function reduceDeploy(rows: StepRow[], e: DeployEvent): StepRow[] {
  switch (e.type) {
    case 'start': {
      // Aynı kimlikli okuma adımı (sağlık yoklaması, inspect) yeniden başlayabilir.
      const i = rows.findIndex((r) => r.step.id === e.step.id && r.step.display === e.step.display)
      if (i === -1) return [...rows, { step: e.step, state: 'running', output: [], detail: '' }]
      return rows.map((r, j) => (j === i ? { ...r, state: 'running', detail: '' } : r))
    }
    case 'output':
      return patchLast(rows, e.id, (r) => ({ ...r, output: [...r.output, e.line].slice(-MAX_OUTPUT) }))
    case 'done':
      return patchLast(rows, e.id, (r) => ({ ...r, state: e.ok ? 'ok' : 'failed', detail: e.detail }))
    case 'skipped':
      return patchLast(rows, e.id, (r) => ({ ...r, state: 'skipped' }))
    case 'note':
      return rows
  }
}

function patchLast(rows: StepRow[], id: number, f: (r: StepRow) => StepRow): StepRow[] {
  let i = -1
  rows.forEach((r, j) => {
    if (r.step.id === id) i = j
  })
  return i === -1 ? rows : rows.map((r, j) => (j === i ? f(r) : r))
}

/** Varsayılan onaylar: değiştiren ve yıkıcı olmayan adımlar; yıkıcılar kullanıcı işaretleyene kadar kapalı. */
export function defaultApproved(plan: DeployPlan): number[] {
  return plan.steps.filter((s) => s.approve && !s.destructive).map((s) => s.id)
}

export function shortSha(sha: string | null | undefined): string {
  return sha ? sha.slice(0, 12) : '–'
}

export function riskCounts(risks: Risk[]): { high: number; medium: number; info: number } {
  return {
    high: risks.filter((r) => r.level === 'high').length,
    medium: risks.filter((r) => r.level === 'medium').length,
    info: risks.filter((r) => r.level === 'info').length,
  }
}

export function when(unixSecs: number): string {
  if (!unixSecs) return '–'
  return new Date(unixSecs * 1000).toLocaleString()
}
