// ipc.ts `triggers` ad alanını tiplemiyor; tipler ipc-triggers.ts içinde.
import type { TriggersApi } from '../../../ipc-triggers.ts'
import { api } from '../../api.ts'

export const triggersApi = (api as unknown as { triggers: TriggersApi }).triggers
