// Görsel girdi köprüsü: KorApi `visual` ad alanını tiplemez (ipc.ts'e dokunulmaz), tipli erişim burada.
import type { VisualApi } from '../../../ipc-visual.ts'
import { api } from '../../api.ts'

export const visualApi = (api as unknown as { visual: VisualApi }).visual
