// Uyumluluk köprüsü: KorApi `compat` ad alanını tiplemez (ipc.ts'e dokunulmaz), tipli erişim burada.
import type { CompatApi } from '../../../ipc-compat.ts'
import { api } from '../../api.ts'

export const compatApi = (api as unknown as { compat: CompatApi }).compat
