// "Doğrudan" sekmesi: tek bir fal / Hugging Face modelini parametre kartı olmadan çalıştırır.
// CLI: kor media models <arama> | kor media run <model> | kor media image | kor hf infer.
import { Play, Search } from 'lucide-react'
import { useId, useState } from 'react'
import type { MediaRunResult } from '../../../ipc.gen.ts'
import { errorMessage } from '../../api.ts'
import { mediaApi } from './api.ts'

type Mode = 'search' | 'run' | 'image' | 'infer'

const MODES: readonly { id: Mode; label: string }[] = [
  { id: 'search', label: 'fal modeli ara' },
  { id: 'run', label: 'fal modelini çalıştır' },
  { id: 'image', label: 'fal ile görsel' },
  { id: 'infer', label: 'Hugging Face çıkarımı' },
]

const HF_TASKS = ['text-generation', 'summarization', 'translation', 'text-classification', 'fill-mask', 'question-answering'] as const

const MAX_INPUT = 20_000

function runText(r: MediaRunResult): string {
  if ('missing' in r) return `Eksik girdiler: ${r.missing.map((m) => m.name).join(', ')}`
  return [r.report, ...r.paths].filter(Boolean).join('\n')
}

export function DirectTab() {
  const id = useId()
  const [mode, setMode] = useState<Mode>('search')
  const [model, setModel] = useState('')
  const [text, setText] = useState('')
  const [task, setTask] = useState<(typeof HF_TASKS)[number]>('text-generation')
  const [busy, setBusy] = useState(false)
  const [out, setOut] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const needsModel = mode === 'run' || mode === 'infer'
  const ready = !busy && text.length <= MAX_INPUT && (!needsModel || model.trim() !== '') && (mode === 'run' || text.trim() !== '')

  const go = async () => {
    setBusy(true)
    setError(null)
    setOut(null)
    try {
      if (mode === 'search') {
        const r = await mediaApi.falModels(text.trim(), null, null)
        setOut('models' in r ? r.models.map((m) => m.id).join('\n') || 'Sonuç yok.' : JSON.stringify(r.schema, null, 2))
      } else if (mode === 'run') {
        let input: object = {}
        if (text.trim()) {
          const parsed: unknown = JSON.parse(text)
          if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) throw new Error('Girdi bir JSON nesnesi olmalı.')
          input = parsed
        }
        setOut(runText(await mediaApi.falRun(model.trim(), input)))
      } else if (mode === 'image') {
        setOut(runText(await mediaApi.falImage(text.trim(), model.trim() || null)))
      } else {
        setOut((await mediaApi.hfInfer(model.trim(), task, text.trim(), null)).text)
      }
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const cmd = mode === 'search' ? 'media.falModels' : mode === 'run' ? 'media.falRun' : mode === 'image' ? 'media.falImage' : 'media.hfInfer'
  return (
    <section className="km-stack" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="km-h">Doğrudan model çalıştır</h3>
      <div className="km-chips" role="radiogroup" aria-label="İşlem">
        {MODES.map((m) => (
          <button key={m.id} type="button" role="radio" aria-checked={mode === m.id} className={`km-chip${mode === m.id ? ' on' : ''}`} onClick={() => setMode(m.id)}>
            {m.label}
          </button>
        ))}
      </div>
      {mode !== 'search' && (
        <label className="km-field">
          <span>{mode === 'infer' ? 'Hugging Face modeli' : mode === 'image' ? 'fal modeli (boş: varsayılan)' : 'fal modeli'}</span>
          <input value={model} onChange={(e) => setModel(e.target.value)} maxLength={200} spellCheck={false} translate="no" placeholder={mode === 'infer' ? 'org/model' : 'fal-ai/…'} />
        </label>
      )}
      {mode === 'infer' && (
        <label className="km-field">
          <span>Görev</span>
          <select className="g7-select" value={task} onChange={(e) => setTask(e.target.value as (typeof HF_TASKS)[number])}>
            {HF_TASKS.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
        </label>
      )}
      <label className="km-field">
        <span>{mode === 'search' ? 'Arama' : mode === 'run' ? 'Girdi (JSON nesnesi)' : mode === 'image' ? 'İstem' : 'Girdi'}</span>
        <textarea value={text} onChange={(e) => setText(e.target.value)} rows={mode === 'search' ? 1 : 5} maxLength={MAX_INPUT} spellCheck={mode !== 'run'} />
      </label>
      <div className="km-row">
        <button type="button" className="btn small" disabled={!ready} onClick={() => void go()} data-kor-cmd="media.falModels media.falRun media.falImage media.hfInfer" data-kor-active={cmd}>
          {mode === 'search' ? <Search size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />} {busy ? 'Çalışıyor…' : 'Çalıştır'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {out !== null && <pre className="km-out" translate="no">{out}</pre>}
    </section>
  )
}
