// `graph.*` ve `packages.*` ad alanları KorApi'de tiplenmedi (ipc.ts paylaşılan); tipler kendi ipc dosyalarında.
import type { GraphApi } from '../../../ipc-graph.ts'
import type { PackagesApi } from '../../../ipc-packages.ts'
import { api } from '../../api.ts'

export const graphApi = (api as unknown as { graph: GraphApi }).graph
export const packagesApi = (api as unknown as { packages: PackagesApi }).packages
