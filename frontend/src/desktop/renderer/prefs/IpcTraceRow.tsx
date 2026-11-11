// `kor doctor --ipc-trace` on the desktop (§17.2, ADR 0061): every bridge call, result and event with its
// timing goes to an NDJSON file (schema violations marked next to the message), for debugging.
import { Activity } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { CoreApi } from '../../ipc.gen.ts'
import { api, errorMessage } from '../api.ts'

const core = (api as unknown as { core: Pick<CoreApi, 'ipcTrace'> }).core

export function IpcTraceRow() {
  const state = useSWR('core:ipcTrace', () => core.ipcTrace(), { revalidateOnFocus: false })
  const [error, setError] = useState<string | null>(null)
  const toggle = async (on: boolean) => {
    setError(null)
    try {
      await state.mutate(await core.ipcTrace(on), { revalidate: false })
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const on = state.data?.enabled ?? false
  return (
    <div className="g7-setting" data-kor-cmd="core.ipcTrace">
      <div className="g7-setting-text">
        <span className="g7-setting-title">IPC izi</span>
        <span className="g7-note">
          kor doctor --ipc-trace: her köprü çağrısı, sonucu ve olayı süresiyle bir dosyaya yazılır; şemaya uymayan iletiler işaretlenir.
          {state.data?.path && (
            <>
              {' '}
              <code translate="no">{state.data.path}</code>
            </>
          )}
        </span>
        {(error ?? (state.error ? errorMessage(state.error) : null)) && <span className="error-text" role="alert">{error ?? errorMessage(state.error)}</span>}
      </div>
      <div className="g7-setting-control">
        <button type="button" className={`btn${on ? ' primary' : ''}`} aria-pressed={on} disabled={!state.data} onClick={() => void toggle(!on)}>
          <Activity size={14} aria-hidden="true" /> {on ? 'Durdur' : 'Başlat'}
        </button>
      </div>
    </div>
  )
}
