// Routines (zamanlanmış görevler) ayar bölümü — cdesktop "Routines" ekranına karşılık.
// Backend (kalıcılık + zamanlayıcı + firing) main.ts'te zaten çalışır; burası yönetim arayüzü.
import { Clock, Play, Plus, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { Trigger } from '../../core/triggers.ts'
import { api, errorMessage } from './api.ts'
import { displayPath } from './display-path.ts'
import { uiLocale } from './ui-locale.ts'

const PRESETS: ReadonlyArray<{ label: string; minutes: number }> = [
  { label: 'Saatlik', minutes: 60 },
  { label: 'Günlük', minutes: 1440 },
  { label: 'Haftalık', minutes: 10_080 },
]

function intervalLabel(minutes: number): string {
  if (minutes % 10_080 === 0) return `${minutes / 10_080} haftada bir`
  if (minutes % 1440 === 0) return `${minutes / 1440} günde bir`
  if (minutes % 60 === 0) return `${minutes / 60} saatte bir`
  return `${minutes} dakikada bir`
}

export function RoutinesSection({ root }: { root: string | null }) {
  const [routines, setRoutines] = useState<Trigger[]>([])
  const [name, setName] = useState('')
  const [prompt, setPrompt] = useState('')
  const [minutes, setMinutes] = useState(1440)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = () =>
    void api.routines
      .list()
      .then(setRoutines)
      .catch((e: unknown) => setError(errorMessage(e)))

  useEffect(refresh, [])

  const nextRunText = useMemo(
    () => (t: Trigger) => (t.enabled ? new Date(t.nextRun).toLocaleString(uiLocale()) : 'Duraklatıldı'),
    [],
  )

  const add = async () => {
    setError(null)
    if (prompt.trim().length === 0) {
      setError('Rutin talimatı boş olamaz.')
      return
    }
    if (!root) {
      setError('Önce bir çalışma klasörü açın; rutin o klasörde çalışır.')
      return
    }
    setBusy(true)
    try {
      await api.routines.add({ name: name.trim() || undefined, prompt: prompt.trim(), intervalMinutes: minutes, root })
      setName('')
      setPrompt('')
      refresh()
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const toggle = (t: Trigger) =>
    void api.routines.setEnabled(t.id, !t.enabled).then(refresh).catch((e: unknown) => setError(errorMessage(e)))
  const remove = (t: Trigger) =>
    void api.routines.remove(t.id).then(refresh).catch((e: unknown) => setError(errorMessage(e)))

  return (
    <div className="routines">
      <p className="g7-section-desc">
        Rutinler, belirlediğin aralıkta kendi başına çalışan görevlerdir. Yalnızca uygulama açıkken çalışır.
      </p>

      <div className="routine-form">
        <label htmlFor="routine-name">Ad (isteğe bağlı)</label>
        <input id="routine-name" className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Örn. Trend repoları çek" maxLength={120} />

        <label htmlFor="routine-prompt">Talimat</label>
        <textarea
          id="routine-prompt"
          className="dialog-textarea routine-prompt"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Ajana ne yapmasını istediğini yaz…"
          maxLength={10_000}
        />

        <label>Çalışma klasörü</label>
        <p className="routine-folder">{root ? displayPath(root) : 'Açık klasör yok — önce bir klasör aç.'}</p>

        <label>Zamanlama</label>
        <div className="routine-presets">
          {PRESETS.map((p) => (
            <button
              key={p.minutes}
              type="button"
              className={`btn small${minutes === p.minutes ? ' primary' : ''}`}
              onClick={() => setMinutes(p.minutes)}
            >
              {p.label}
            </button>
          ))}
          <span className="routine-custom">
            her
            <input
              type="number"
              className="text-input routine-min-input"
              min={1}
              value={minutes}
              onChange={(e) => setMinutes(Math.max(1, Number(e.target.value) || 1))}
            />
            dk
          </span>
        </div>

        {error && <p className="routine-error">{error}</p>}

        <div className="routine-actions">
          <button type="button" className="btn primary" onClick={() => void add()} disabled={busy}>
            <Plus size={14} aria-hidden="true" /> Rutin ekle
          </button>
        </div>
      </div>

      <h4 className="g7-h4">Mevcut rutinler</h4>
      {routines.length === 0 ? (
        <p className="muted">Henüz rutin yok.</p>
      ) : (
        <ul className="routine-list">
          {routines.map((t) => (
            <li key={t.id} className={`routine-item${t.enabled ? '' : ' off'}`}>
              <span className="routine-ic">
                <Clock size={16} aria-hidden="true" />
              </span>
              <span className="routine-text">
                <span className="routine-name">{t.name}</span>
                <span className="routine-meta">
                  {intervalLabel(t.intervalMinutes)} · Sonraki: {nextRunText(t)}
                </span>
              </span>
              <button type="button" className="btn small" onClick={() => toggle(t)} title={t.enabled ? 'Duraklat' : 'Sürdür'}>
                <Play size={13} aria-hidden="true" /> {t.enabled ? 'Duraklat' : 'Sürdür'}
              </button>
              <button type="button" className="action-btn routine-del" onClick={() => remove(t)} aria-label="Rutini sil" title="Sil">
                <Trash2 size={15} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
