// ipc.ts `deploy`, `watch` ve §8.5 `db` eklerini tiplemiyor; tipler ipc-deploy.ts içinde.
import type { DbOpsApi, DeployApi, WatchApi } from '../../../ipc-deploy.ts'
import { api } from '../../api.ts'

const kor = api as unknown as { deploy: DeployApi; watch: WatchApi; db: DbOpsApi }

export const deployApi = kor.deploy
export const watchApi = kor.watch
export const dbOpsApi = kor.db
