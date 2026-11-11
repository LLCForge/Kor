// ipc.ts `metrics` ad alanını tiplemiyor; tipler ipc-metrics.ts içinde, erişim tek yerde.
import type { MetricsApi } from '../../../ipc-metrics.ts'
import { api } from '../../api.ts'

export const metricsApi = (api as unknown as { metrics: MetricsApi }).metrics
