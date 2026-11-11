// ipc.ts `tasks` ad alanını tiplemiyor; tipler ipc-tasks.ts içinde.
import type { TasksApi } from '../../../ipc-tasks.ts'
import { api } from '../../api.ts'

export const tasksApi = (api as unknown as { tasks: TasksApi }).tasks
