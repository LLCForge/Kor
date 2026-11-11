// ipc.ts `analysis` ad alanını tiplemiyor; tipler ipc-analysis.ts içinde, erişim tek yerde.
import type { AnalysisApi } from '../../../ipc-analysis.ts'
import { api } from '../../api.ts'

export const analysisApi = (api as unknown as { analysis: AnalysisApi }).analysis
