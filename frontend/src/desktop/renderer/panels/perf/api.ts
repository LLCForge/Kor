// KorApi `perf` ad alanını tiplemez (ipc.ts'e dokunulmaz); tipli erişim burada.
import type { PerfApi } from '../../../ipc-perf.ts'
import { api } from '../../api.ts'

export const perfApi = (api as unknown as { perf: PerfApi }).perf
