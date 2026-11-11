// [g10] Güvenilirlik katmanı: yarım tur kurtarma bantları, ölü mektup kuyruğu, başlangıç öz-denetimi ve kısa olay bildirimleri.
import { Inbox, RotateCcw, ShieldAlert, TriangleAlert, X } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { CheckpointInfo } from '../ipc-g4.ts'
import type { G10Event, RecoverableTurn } from '../ipc-g10.ts'
import { api, errorMessage } from './api.ts'
import type { ChatAction, ChatState, FileChange } from './chat-state.ts'
import { RewindDialog } from './RewindDialog.tsx'
import { uiLocale } from './ui-locale.ts'

interface G10LayerProps {
  activeId: string
  chat: ChatState
  dispatchStore: (action: { sessionId: string; action: ChatAction }) => void
  /** App'in sohbet açma akışı (döküm yükleme, klasör değişimi) */
  onOpenSession: (id: string) => Promise<void>
}

interface Toast {
  id: string
  text: string
}

const TOAST_MS = 8_000
const isFsKey = (key: unknown) => Array.isArray(key) && (key[0] === 'dir' || key[0] === 'file')

export function describeG10Event(e: G10Event): string {
  switch (e.kind) {
    case 'circuitOpen':
      return `"${e.target}" art arda hata verdi; yaklaşık ${e.retryInSec} sn devre dışı.`
    case 'toolTimeout':
      return `${e.tool} ${e.seconds} sn içinde bitmedi ve iptal edildi: ${e.reason}`
    case 'reconnect':
      return `Akış bağlantısı koptu; yeniden bağlanılıyor (${e.attempt}. deneme${e.continued ? ', kaldığı yerden' : ''}).`
    case 'degraded':
      return `${e.from} ${e.seconds} sn içinde yanıt vermedi; bu adım için yedek model ${e.to} kullanılıyor.`
    case 'contextTrimmed':
      return e.ok ? 'Bağlam sınırı aşıldı; eski turlar özetlenip istek yeniden gönderildi.' : 'Bağlam sınırı aşıldı ve özetlenecek eski tur kalmadı.'
    case 'jsonRepaired':
      return `${e.tool} çağrısının bozuk argümanları onarıldı (${e.fixes.join(', ')}).`
    case 'refusal':
      return 'Model isteği reddetti. Alternatif: isteği bağlamıyla yeniden ifade et, görevi böl veya başka bir model seç.'
    case 'deadLetter':
      return `Arka plan görevi başarısız: ${e.title}`
    case 'conflict':
      return `${e.path} son okunmasından sonra dışarıda değişmiş; ezilmedi.`
    case 'checkpoint':
      return `Riskli komut öncesi ${e.files} dosyanın hali kaydedildi${e.partial ? ' (sınır nedeniyle kısmi)' : ''}.`
  }
}

const shortTime = (ms: number) => new Date(ms).toLocaleString(uiLocale(), { dateStyle: 'short', timeStyle: 'short' })

export function G10Layer({ activeId, chat, dispatchStore, onOpenSession }: G10LayerProps) {
  const { mutate } = useSWRConfig()
  const recoverable = useSWR('g10:recoverable', () => api.g10.listRecoverable(), { revalidateOnFocus: false })
  const selfCheck = useSWR('g10:selfcheck', () => api.g10.selfCheck(), { revalidateOnFocus: false })
  const deadLetters = useSWR('g10:dlq', () => api.g10.listDeadLetters(), { revalidateOnFocus: false })
  const [gitBusy, setGitBusy] = useState(false)
  const [gitNote, setGitNote] = useState<string | null>(null)
  const installGit = () => {
    setGitBusy(true)
    setGitNote(null)
    api.g10
      .installGit()
      .then((r) => {
        setGitNote(r.message)
        if (r.ok) void selfCheck.mutate()
      })
      .catch(() => setGitNote('Kurulum başlatılamadı.'))
      .finally(() => setGitBusy(false))
  }
  const last = chat.items.at(-1)
  const errorId = !chat.running && last?.kind === 'error' ? last.id : null
  const recovery = useSWR(errorId ? ['g10:recovery', activeId, errorId] : null, () => api.g10.recoveryInfo(activeId), { revalidateOnFocus: false })

  const [toasts, setToasts] = useState<Toast[]>([])
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const [panel, setPanel] = useState<'dlq' | 'check' | null>(null)
  const [rewind, setRewind] = useState<CheckpointInfo | null>(null)
  const panelId = useId()
  const activeRef = useRef(activeId)
  activeRef.current = activeId
  const refreshDlq = deadLetters.mutate

  // [cdx] Kullanıcı mesajındaki "geri sar" düğmesi ve genel bildirimler pencere olayıyla gelir.
  useEffect(() => {
    const onRewind = (e: Event) => setRewind((e as CustomEvent<CheckpointInfo>).detail)
    const onToast = (e: Event) => {
      const id = crypto.randomUUID()
      setToasts((prev) => [...prev.slice(-2), { id, text: String((e as CustomEvent<string>).detail ?? '') }])
      setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), TOAST_MS)
    }
    window.addEventListener('cdx:rewind', onRewind)
    window.addEventListener('cdx:toast', onToast)
    return () => {
      window.removeEventListener('cdx:rewind', onRewind)
      window.removeEventListener('cdx:toast', onToast)
    }
  }, [])
  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.type !== 'g10:event') return
        if (event.event.kind === 'deadLetter') void refreshDlq()
        else if (event.sessionId !== activeRef.current) return
        const id = crypto.randomUUID()
        setToasts((prev) => [...prev.slice(-2), { id, text: describeG10Event(event.event) }])
        setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), TOAST_MS)
      }),
    [refreshDlq],
  )

  // Bandı kapatmak ona bağlı açık ayrıntı panelini de kapatır.
  const hide = (key: string) => {
    setHidden((prev) => new Set(prev).add(key))
    setPanel((p) => (p === key ? null : p))
  }

  const run = async (task: () => Promise<void>) => {
    setBusy(true)
    setFailure(null)
    try {
      await task()
    } catch (err) {
      setFailure(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const sendAs = async (sessionId: string, text: string) => {
    dispatchStore({ sessionId, action: { type: 'userSent', text, attachments: [], at: Date.now() } })
    await api.setActiveSession(sessionId)
    await api.send(sessionId, text, [])
  }

  const resume = (turn: RecoverableTurn) =>
    run(async () => {
      if (turn.crashed) await onOpenSession(turn.sessionId)
      // Hata sonrası kurtarmada değişiklik paneli korunur; 'load' onu sıfırlar.
      const keep: FileChange[] = turn.crashed ? [] : chat.changes
      const result = await api.g10.restore(turn.sessionId)
      dispatchStore({ sessionId: turn.sessionId, action: { type: 'load', transcript: result.transcript } })
      if (keep.length > 0) dispatchStore({ sessionId: turn.sessionId, action: { type: 'rewound', changes: keep, cutFromItemId: null } })
      await sendAs(turn.sessionId, result.prompt)
      await Promise.all([recoverable.mutate(), recovery.mutate()])
    })

  const dismiss = (turn: RecoverableTurn) =>
    run(async () => {
      hide(`turn:${turn.sessionId}:${turn.startedAt}`)
      await api.g10.dismiss(turn.sessionId)
    })

  /** Turdaki tüm dosya değişiklikleri tek birim: g4'ün bu tur için açtığı kontrol noktasına geri sarılır. */
  const rollback = (turn: RecoverableTurn) =>
    run(async () => {
      const latest = (await api.g4.listCheckpoints(turn.sessionId)).at(-1)
      const covers = latest && latest.available && turn.files.every((f) => latest.files.includes(f))
      if (!latest || !covers) throw new Error('Bu tur için kullanılabilir bir kontrol noktası yok; dosyaları değişiklik panelinden tek tek geri alabilirsin.')
      setRewind(latest)
    })

  const retryDeadLetter = (id: string) =>
    run(async () => {
      const { sessionId, prompt } = await api.g10.retryDeadLetter(id)
      if (sessionId !== activeRef.current) await onOpenSession(sessionId)
      await sendAs(sessionId, prompt)
      await deadLetters.mutate()
    })

  const removeDeadLetter = (id: string) =>
    run(async () => {
      await api.g10.removeDeadLetter(id)
      await deadLetters.mutate()
    })

  const crashed = (recoverable.data ?? []).filter((t) => !hidden.has(`turn:${t.sessionId}:${t.startedAt}`))
  const failed = recovery.data && errorId && !hidden.has(`error:${errorId}`) ? recovery.data : null
  const problems = (selfCheck.data ?? []).filter((c) => c.status !== 'ok')
  const letters = deadLetters.data ?? []

  return (
    <>
      <div className="g10-bands" aria-live="polite">
        {crashed.map((turn) => (
          <div key={`${turn.sessionId}:${turn.startedAt}`} className="g10-band pop-in" role="status">
            <RotateCcw size={14} aria-hidden="true" />
            <span>
              Yarıda kalan tur: <strong>{turn.title}</strong> ({turn.steps} adım tamamlanmıştı)
            </span>
            <button type="button" className="btn small primary" disabled={busy} onClick={() => void resume(turn)}>
              Devam et
            </button>
            <button type="button" className="btn small ghost" disabled={busy} onClick={() => void dismiss(turn)}>
              Yoksay
            </button>
          </div>
        ))}

        {failed && errorId && (
          <div className="g10-band pop-in" role="status">
            <TriangleAlert size={14} aria-hidden="true" />
            <span>
              Tur yarıda kaldı: {failed.steps} adım uygulanmıştı
              {failed.files.length > 0 ? ` (${failed.files.slice(0, 3).join(', ')}${failed.files.length > 3 ? '…' : ''})` : ''}.
            </span>
            <button type="button" className="btn small primary" disabled={busy} onClick={() => void resume(failed)}>
              Kaldığı yerden devam et
            </button>
            {failed.files.length > 0 && (
              <button type="button" className="btn small" disabled={busy} onClick={() => void rollback(failed)}>
                Değişiklikleri geri al
              </button>
            )}
            <button type="button" className="btn small ghost" aria-label="Kapat" onClick={() => hide(`error:${errorId}`)}>
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        )}

        {letters.length > 0 && !hidden.has('dlq') && (
          <div className="g10-band pop-in" role="status">
            <Inbox size={14} aria-hidden="true" />
            <span>{letters.length} başarısız arka plan görevi</span>
            <button
              type="button"
              className="btn small"
              aria-expanded={panel === 'dlq'}
              aria-controls={`${panelId}-dlq`}
              onClick={() => setPanel((p) => (p === 'dlq' ? null : 'dlq'))}
            >
              İncele
            </button>
            <button type="button" className="btn small ghost" aria-label="Kapat" onClick={() => hide('dlq')}>
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        )}

        {problems.length > 0 && !hidden.has('check') && (
          <div className={`g10-band pop-in${problems.some((p) => p.status === 'fail') ? ' fail' : ' warn'}`} role="status">
            <ShieldAlert size={14} aria-hidden="true" />
            <span>Başlangıç denetimi: {problems.length} sorun</span>
            <button
              type="button"
              className="btn small"
              aria-expanded={panel === 'check'}
              aria-controls={`${panelId}-check`}
              onClick={() => setPanel((p) => (p === 'check' ? null : 'check'))}
            >
              Ayrıntılar
            </button>
            <button type="button" className="btn small ghost" aria-label="Kapat" onClick={() => hide('check')}>
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        )}

        {panel === 'dlq' && letters.length > 0 && (
          <section id={`${panelId}-dlq`} className="g10-panel pop-in" aria-label="Başarısız arka plan görevleri">
            <ul>
              {letters.map((d) => (
                <li key={d.id}>
                  <div className="g10-panel-head">
                    <strong>{d.kind === 'subagent' ? 'Alt ajan' : 'Komut'}</strong>
                    <span className="g10-muted">{shortTime(d.at)}</span>
                  </div>
                  <code className="g10-title">{d.title}</code>
                  <p className="g10-error">{d.error.slice(0, 400)}</p>
                  <div className="g10-actions">
                    <button type="button" className="btn small" disabled={busy} onClick={() => void retryDeadLetter(d.id)}>
                      Yeniden dene
                    </button>
                    <button type="button" className="btn small ghost" disabled={busy} onClick={() => void removeDeadLetter(d.id)}>
                      Kaldır
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          </section>
        )}

        {panel === 'check' && (
          <section id={`${panelId}-check`} className="g10-panel pop-in" aria-label="Başlangıç denetimi sonuçları">
            <ul>
              {(selfCheck.data ?? []).map((c) => (
                <li key={c.id} className={`g10-check ${c.status}`}>
                  <strong>{c.label}</strong>
                  <span className="g10-status">{c.status === 'ok' ? 'Tamam' : c.status === 'warn' ? 'Uyarı' : 'Sorun'}</span>
                  <span>{c.message}</span>
                  {c.id === 'git' && c.status !== 'ok' && (
                    <button type="button" className="btn small g10-check-fix" disabled={gitBusy} onClick={installGit}>
                      {gitBusy ? 'Kuruluyor…' : 'Otomatik kur'}
                    </button>
                  )}
                </li>
              ))}
            </ul>
            {gitNote && <p className="g10-note" role="status">{gitNote}</p>}
            <div className="g10-actions">
              <button type="button" className="btn small" disabled={selfCheck.isValidating} onClick={() => void selfCheck.mutate()}>
                Yeniden denetle
              </button>
            </div>
          </section>
        )}

        {failure && (
          <div className="g10-band fail pop-in" role="alert">
            <TriangleAlert size={14} aria-hidden="true" />
            <span>{failure}</span>
            <button type="button" className="btn small ghost" aria-label="Kapat" onClick={() => setFailure(null)}>
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        )}

        {toasts.map((t) => (
          <div key={t.id} className="g10-band toast pop-in" role="status">
            <span>{t.text}</span>
          </div>
        ))}
      </div>

      {rewind && (
        <RewindDialog
          sessionId={activeId}
          checkpointId={rewind.id}
          text={rewind.text}
          onClose={() => setRewind(null)}
          onDone={(result) => {
            const changes: FileChange[] = result.changes.map((c) => ({ path: c.path, before: c.before, after: c.after ?? '' }))
            dispatchStore({ sessionId: activeId, action: { type: 'rewound', changes, cutFromItemId: null } })
            if (errorId) hide(`error:${errorId}`)
            void api.g10.dismiss(activeId).catch((err: unknown) => console.error('Kurtarma kaydı silinemedi:', err))
            void mutate(isFsKey)
            void mutate(['g4:checkpoints', activeId])
          }}
        />
      )}
    </>
  )
}
