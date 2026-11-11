import type { Approval } from '../../../core/agent.ts'
import type { FileDecisions, ReviewFile } from '../../ipc-review.ts'

/** Arayüzdeki dosya kararı: kabul, ret ya da kullanıcının düzenlediği içerikle kabul. */
export type Choice = { kind: 'accept' } | { kind: 'reject' } | { kind: 'edit'; content: string }

export type Choices = Readonly<Record<string, Choice>>

export type ReviewAction =
  | { type: 'set'; path: string; choice: Choice }
  | { type: 'all'; kind: 'accept' | 'reject' }

/** Başlangıçta her dosya önerildiği gibi kabul edilir. */
export function initialChoices(files: readonly ReviewFile[]): Choices {
  return Object.fromEntries(files.map((f) => [f.path, { kind: 'accept' } as Choice]))
}

export function reviewReducer(state: Choices, action: ReviewAction): Choices {
  switch (action.type) {
    case 'set':
      return { ...state, [action.path]: action.choice }
    case 'all':
      return Object.fromEntries(Object.keys(state).map((p) => [p, { kind: action.kind } as Choice]))
  }
}

/** Gösterilecek "sonra" içeriği: düzenlenmişse kullanıcınınki. */
export function shownAfter(file: ReviewFile, choice: Choice | undefined): string | null {
  return choice?.kind === 'edit' ? choice.content : file.after
}

/**
 * Çekirdeğe giden yanıt. Hepsi kabul → düz 'yes' (eski yol, araç olduğu gibi çalışır);
 * hepsi ret → 'no'; karışık ya da düzenlenmiş → 'yes' + dosya kararları.
 */
export function toAnswer(files: readonly ReviewFile[], choices: Choices): { approval: Approval; files?: FileDecisions } {
  const list = files.map((f) => ({ f, c: choices[f.path] ?? ({ kind: 'accept' } as Choice) }))
  const edited = (x: { f: ReviewFile; c: Choice }) => x.c.kind === 'edit' && x.c.content !== x.f.after
  if (list.every((x) => x.c.kind === 'reject')) return { approval: 'no' }
  if (list.every((x) => x.c.kind !== 'reject' && !edited(x))) return { approval: 'yes' }
  const out: FileDecisions = {}
  for (const x of list) out[x.f.path] = x.c.kind === 'reject' ? 'reject' : edited(x) && x.c.kind === 'edit' ? { edit: x.c.content } : 'accept'
  return { approval: 'yes', files: out }
}

export function counts(files: readonly ReviewFile[], choices: Choices): { accepted: number; rejected: number; edited: number } {
  let accepted = 0
  let rejected = 0
  let edited = 0
  for (const f of files) {
    const c = choices[f.path]
    if (c?.kind === 'reject') rejected++
    else if (c?.kind === 'edit') edited++
    else accepted++
  }
  return { accepted, rejected, edited }
}
