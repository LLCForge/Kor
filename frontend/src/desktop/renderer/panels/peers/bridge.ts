// `peer.*` ve `acp.*` ad alanları ipc.gen.ts'te tipli (ADR 0061); KorApi henüz içermediği için tip dönüşümü.
import type { AcpApi, PeerApi } from '../../../ipc.gen.ts'
import { api } from '../../api.ts'

export const peerApi = (api as unknown as { peer: PeerApi }).peer
export const acpApi = (api as unknown as { acp: AcpApi }).acp
