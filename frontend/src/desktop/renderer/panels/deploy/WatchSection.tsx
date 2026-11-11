// `kor watch logs` (§8.7): kaynak izleme, olaylar, onayla analiz → düzeltme dalı → pull request.
import { Eye, GitPullRequest, Square, Trash2, Wrench } from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { Incident } from '../../../ipc-deploy.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { insertIntoDraft } from '../../g21/quote.ts'
import { watchApi } from './bridge.ts'
import { when } from './shape.ts'

export function WatchSection() {
  const watchers = useSWR('watch:list', () => watchApi.list(), { revalidateOnFocus: false })
  const incidents = useSWR('watch:incidents', () => watchApi.incidents(), { revalidateOnFocus: false })
  const [source, setSource] = useState('')
  const [notes, setNotes] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  useEffect(
    () =>
      watchApi.onEvent(({ event }) => {
        if (event.type === 'incident' || event.type === 'repeat') void incidents.mutate()
        if (event.type === 'listening') setNotes((n) => [...n, `Webhook adresi (bir tünelle dışarı açın): ${event.url}`])
        if (event.type === 'limited') setNotes((n) => [...n, `Çok fazla yeni hata: ${event.total} tanesi tek tek bildirilmedi.`])
        if (event.type === 'ended') {
          if (event.error) setError(event.error)
          void watchers.mutate()
        }
      }),
    [incidents, watchers],
  )

  const start = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await watchApi.start(source.trim())
      setSource('')
      void watchers.mutate()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  const act = async (key: string, f: () => Promise<void>) => {
    setBusy(key)
    setError(null)
    try {
      await f()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
      void incidents.mutate()
    }
  }

  const fix = (i: Incident) =>
    act(`fix:${i.id}`, async () => {
      const ok = await confirmAction({
        title: 'Kök nedeni bul ve düzelt',
        message: `kor/fix-${i.id} dalı açılır ve ajanın görevi mesaj kutusuna yazılır. Gönderdiğinizde ajan grafı ve log satırlarını kullanır; her dosya yazımı ayrıca onay ister.`,
        confirmLabel: 'Dalı aç',
      })
      if (!ok) return
      const { prompt } = await watchApi.fixStart(i.id)
      insertIntoDraft(prompt)
    })

  const pr = (i: Incident) =>
    act(`pr:${i.id}`, async () => {
      const files = await watchApi.fixFiles()
      const ok = await confirmAction({
        title: 'Pull request aç',
        message: files.length
          ? `Şu ${files.length} dosya commit'lenip ${i.fix?.branch ?? 'dal'} gönderilecek ve taslak pull request açılacak:\n${files.join('\n')}`
          : 'Yeni değişiklik yok; daldaki commit\'ler gönderilip taslak pull request açılacak.',
        confirmLabel: 'Gönder ve aç',
      })
      if (!ok) return
      const r = await watchApi.fixFinish(i.id)
      setNotes((n) => [...n, `Pull request açıldı (${r.forge}): ${r.url}`])
    })

  return (
    <section className="kg-card" aria-label="Log izleme" data-kor-event="kor:watch/event">
      <form className="kg-row" onSubmit={(e) => void start(e)}>
        <label className="kg-field kd-grow">
          <span>Kaynak</span>
          <input value={source} onChange={(e) => setSource(e.target.value)} placeholder="logs/app.log, journald:nginx, docker:web, sentry, datadog" spellCheck={false} maxLength={300} translate="no" required />
        </label>
        <button type="submit" className="btn small" data-kor-cmd="watch.start">
          <Eye size={14} aria-hidden="true" />
          İzle
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {notes.map((n, i) => <p key={i} className="muted kd-small" translate="no">{n}</p>)}
      {(watchers.data ?? []).length > 0 && (
        <ul className="kd-watchers" aria-label="Çalışan izleyiciler" data-kor-cmd="watch.list">
          {(watchers.data ?? []).map((w) => (
            <li key={w.id}>
              <span translate="no">{w.source}</span>
              <button type="button" className="action-btn" onClick={() => void watchApi.stop(w.id).then(() => watchers.mutate())} aria-label="İzlemeyi durdur" title="Durdur" data-kor-cmd="watch.stop">
                <Square size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="kd-inc-head">
        <h4>Görülen hatalar</h4>
        {(incidents.data ?? []).length > 0 && (
          <button type="button" className="action-btn" onClick={() => void act('clear', async () => { await watchApi.clear(null) })} aria-label="Tüm olayları sil" title="Hepsini sil" data-kor-cmd="watch.clear">
            <Trash2 size={14} aria-hidden="true" />
          </button>
        )}
      </div>
      {incidents.error ? (
        <p className="error-text" role="alert">{errorMessage(incidents.error)}</p>
      ) : !incidents.data ? (
        <p className="muted" role="status" data-kor-cmd="watch.incidents">Olaylar okunuyor…</p>
      ) : incidents.data.length === 0 ? (
        <p className="muted" data-kor-cmd="watch.incidents">Henüz hata görülmedi. Bir kaynağı izlemeye başlayın.</p>
      ) : (
        <ul className="kd-incidents" aria-label="Olaylar" data-kor-cmd="watch.incidents">
          {incidents.data.map((i) => (
            <li key={i.id}>
              <details>
                <summary>
                  <span className="kd-count">{`${i.count}×`}</span>
                  <span translate="no">{i.title}</span>
                </summary>
                <p className="muted kd-small" translate="no">{`${i.source} · ${when(i.last_at)} · ${i.id}`}</p>
                <pre className="kd-out" translate="no">{i.context.join('\n')}</pre>
              </details>
              <div className="kg-row">
                <button type="button" className="btn small" disabled={busy !== null} onClick={() => void fix(i)} data-kor-cmd="watch.fixStart">
                  <Wrench size={14} aria-hidden="true" />
                  Analiz et ve düzelt
                </button>
                {i.fix && !i.fix.pr && (
                  <button type="button" className="btn small" disabled={busy !== null} onClick={() => void pr(i)} data-kor-cmd="watch.fixFiles watch.fixFinish">
                    <GitPullRequest size={14} aria-hidden="true" />
                    Pull request aç
                  </button>
                )}
                {i.fix?.pr && <span className="muted kd-small" translate="no">{i.fix.pr}</span>}
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
