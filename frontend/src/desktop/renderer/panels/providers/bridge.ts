// ipc.ts bu ad alanlarını tiplemiyor; tipler ipc-providers.ts'te, erişim tek yerde.
import type { ProvidersPanelApi } from '../../../ipc-providers.ts'
import { api } from '../../api.ts'

export const kor = api as unknown as ProvidersPanelApi
