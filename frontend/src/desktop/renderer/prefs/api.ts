// KorApi `prefs`, `plan`, `sessions` ad alanlarını tiplemez (ipc.ts'e dokunulmaz); tipli erişim burada (ADR 0050).
import type { PlanApi, PrefsApi, SessionsApi } from '../../ipc-prefs.ts'
import { api } from '../api.ts'

const typed = api as unknown as { prefs: PrefsApi; plan: PlanApi; sessions: SessionsApi; onContextCompactPreview?: unknown }

export const prefsApi = typed.prefs
export const planApi = typed.plan
export const sessionsApi = typed.sessions
