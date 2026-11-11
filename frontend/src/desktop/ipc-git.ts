// Git paneli IPC sözleşmesi (cdesktop Git kartı): YALNIZCA kanal adları + tipler.
// Preload bu dosyayı import eder (sandbox) → burada electron/node import'u OLMAMALI. Handler: handlers/git.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { GitStatus, GitBranches, CreatePrInput, CreatePrResult, GitApi } from './ipc.gen.ts'
export type { GitStatus, GitBranches, CreatePrInput, CreatePrResult, GitApi }
export const GIT_CHANNELS = {
  status: 'git:status',
  branches: 'git:branches',
  push: 'git:push',
  createPr: 'git:createPr',
  openPr: 'git:openPr',
  merge: 'git:merge',
  rebase: 'git:rebase',
  abortOperation: 'git:abortOperation',
  continueOperation: 'git:continueOperation',
} as const
