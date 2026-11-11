// ipc.ts bu ad alanlarını tiplemiyor; tipler ipc-security.ts / ipc-team.ts / ipc-shell.ts içinde, erişim tek yerde.
import type { AuditApi, SandboxApi, VaultApi } from '../../../ipc-security.ts'
import type { GitflowApi, ShellApi } from '../../../ipc-shell.ts'
import type { TeamApi } from '../../../ipc-team.ts'
import { api } from '../../api.ts'

const kor = api as unknown as { vault: VaultApi; audit: AuditApi; sandbox: SandboxApi; team: TeamApi; shell: ShellApi; gitflow: GitflowApi }

export const vaultApi = kor.vault
export const auditApi = kor.audit
export const sandboxApi = kor.sandbox
export const teamApi = kor.team
export const shellApi = kor.shell
export const gitflowApi = kor.gitflow
