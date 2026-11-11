// K14 köprüleri: KorApi bu ad alanlarını tiplemez (ipc.ts'e dokunulmaz), tipli erişim burada.
import type { DiagramApi, DocsApi, EstimateApi, ForkApi, HooksApi, KnowApi, McpDevApi, RulesApi } from '../ipc-ek14.ts'
import { api } from './api.ts'

const ns = api as unknown as {
  fork: ForkApi
  diagram: DiagramApi
  know: KnowApi
  estimate: EstimateApi
  rules: RulesApi
  docs: DocsApi
  hooks: HooksApi
  mcpdev: McpDevApi
}

export const forkApi = ns.fork
export const diagramApi = ns.diagram
export const knowApi = ns.know
export const estimateApi = ns.estimate
export const rulesApi = ns.rules
export const docsApi = ns.docs
export const hooksApi = ns.hooks
export const mcpDevApi = ns.mcpdev
