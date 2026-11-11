// ipc.ts `prereq` ad alanını tiplemiyor; tipler ipc-prereq.ts içinde.
import type { PrereqApi } from '../../../ipc-prereq.ts'
import { api } from '../../api.ts'

export const prereqApi = (api as unknown as { prereq: PrereqApi }).prereq
