// ipc.ts `media` ad alanının yeni yöntemlerini tiplemiyor; tipler ipc-media.ts'te, erişim tek yerde.
import type { MediaGenApi } from '../../../ipc-media.ts'
import type { MediaApi } from '../../../ipc-providers.ts'
import { api } from '../../api.ts'

export const mediaApi = (api as unknown as { media: MediaGenApi & MediaApi }).media
