// KorApi `seo` ad alanını tiplemez (ipc.ts'e dokunulmaz); tipli erişim burada.
import type { SeoApi } from '../../../../ipc-seo.ts'
import { api } from '../../../api.ts'

export const seoApi = (api as unknown as { seo: SeoApi }).seo
