// Replay ve ortam köprüleri: KorApi `replay`/`env` ad alanlarını tiplemez (ipc.ts'e dokunulmaz), tipli erişim burada.
import type { EnvApi, ReplayApi } from '../../../ipc-replay.ts'
import { api } from '../../api.ts'

export const replayApi = (api as unknown as { replay: ReplayApi }).replay
export const envApi = (api as unknown as { env: EnvApi }).env
