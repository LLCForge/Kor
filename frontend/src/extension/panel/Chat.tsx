// Sohbet (§18.3): oturum seçimi, akan ajan çıktısı, KorStatus fiili, onaylar ve sorular; mesaj gönder, çalışan
// göreve mesaj ekle (kuyruk), durdur, devam et (eski oturuma yaz), slash komutları ve sayfa bağlamı ekleri.
import { useEffect, useRef, useState } from 'react'
import { Send, Square, X } from 'lucide-react'
import { client } from './bus.ts'
import { Approval } from './Approval.tsx'
import { answered, type View } from './events.ts'
import { t } from './i18n.ts'
import { KorSpinner } from 'kor-assets/KorThinking'

export interface Session {
  id: string
  title: string
}

export interface Context {
  url?: string
  title?: string
  text?: string
  selection?: string
  screenshot?: string
}

interface Props {
  sessions: Session[]
  current: string
  view: View
  context: Context | null
  draft: string
  setDraft: (s: string) => void
  onSelect: (id: string) => void
  onSent: (id: string, text: string) => void
  onAnswered: (v: View) => void
  clearContext: () => void
}

export function Chat(p: Props) {
  const log = useRef<HTMLDivElement>(null)
  const [error, setError] = useState('')
  const [slash, setSlash] = useState<Array<{ name: string; about: string }>>([])
  useEffect(() => {
    log.current?.scrollTo({ top: log.current.scrollHeight })
  }, [p.view.items.length, p.view.items[p.view.items.length - 1]])
  useEffect(() => {
    client<{ slash: Array<{ name: string; about: string }> }>('commands').then((c) => setSlash(c.slash), () => setSlash([]))
  }, [])

  const send = () => {
    const text = p.draft.trim()
    if (!text) return
    setError('')
    client<{ sessionId: string }>('chat', { sessionId: p.current, text, context: p.context ?? undefined }).then(
      (r) => {
        p.onSent(r.sessionId, text)
        p.setDraft('')
        p.clearContext()
      },
      (e: Error) => setError(e.message),
    )
  }
  const stop = () => void client('abort', { sessionId: p.current }).catch((e: Error) => setError(e.message))
  const answer = (id: string, choice: number) =>
    void client('answer', { id, value: { choice } }).then(() => p.onAnswered(answered(p.view, id)), (e: Error) => setError(e.message))
  const hints = p.draft.startsWith('/') && !p.draft.includes(' ') ? slash.filter((s) => s.name.startsWith(p.draft.slice(1))).slice(0, 6) : []

  return (
    <section className="chat" aria-label={t('Sohbet')}>
      <div className="row">
        <select aria-label={t('Oturum')} value={p.current} onChange={(e) => p.onSelect(e.target.value)} data-kor-cmd="client.transcript">
          <option value="">{t('Yeni görev')}</option>
          {p.sessions.map((s) => (
            <option key={s.id} value={s.id} translate="no">
              {s.title}
            </option>
          ))}
        </select>
        {p.view.cost ? <span className="pill-text" title={t('Bu görevin maliyeti')}>{p.view.cost}</span> : null}
      </div>
      <div className="log" ref={log} data-kor-event="kor:event">
        {p.view.items.length === 0 ? <p className="note">{t('Kor\'a bir görev yazın; CLI ve masaüstündeki görevler burada da görünür.')}</p> : null}
        {p.view.items.map((it, i) => {
          switch (it.kind) {
            case 'user':
              return <div key={i} className="msg user" translate="no">{it.text}</div>
            case 'assistant':
              return <div key={i} className="msg assistant" translate="no">{it.text}</div>
            case 'tool':
              return (
                <div key={i} className="tool">
                  <span className="nm" translate="no">{it.name}</span> <span translate="no">{it.summary}</span> {it.done ? '✓' : '…'}
                </div>
              )
            case 'confirm':
              return <Approval key={it.ev.id} ev={it.ev} onDone={() => p.onAnswered(answered(p.view, it.ev.id))} />
            case 'question':
              return (
                <div key={it.id} className="confirm" data-kor-cmd="client.answer">
                  <div className="q" translate="no">{it.question}</div>
                  <div className="cmds">
                    {it.options.map((o, n) => (
                      <button key={n} type="button" onClick={() => answer(it.id, n)} translate="no">
                        {o}
                      </button>
                    ))}
                  </div>
                </div>
              )
            case 'notice':
              return <div key={i} className="tool">{it.text}</div>
            case 'error':
              return <div key={i} className="err">{it.text}</div>
          }
        })}
        {p.view.running ? (
          // §18.3/§23: masaüstündeki KorStatus gibi marka spinner'ı + çekirdeğin fiili; fiil değişince kayarak gelir.
          <div className="status-verb" role="status">
            <KorSpinner size={14} />
            <span key={p.view.verb} className="status-verb-text">{p.view.verb || t('Çalışıyor…')}</span>
          </div>
        ) : null}
      </div>
      {p.context ? (
        <div className="chip">
          {t('Bağlam')}: <span translate="no">{p.context.title || p.context.url}</span>
          {p.context.selection ? ` · ${t('seçim')}` : ''}
          {p.context.text ? ` · ${t('sayfa')}` : ''}
          {p.context.screenshot ? ` · ${t('ekran görüntüsü')}` : ''}
          <button type="button" className="icon" aria-label={t('Bağlamı kaldır')} title={t('Bağlamı kaldır')} onClick={p.clearContext}>
            <X size={14} />
          </button>
        </div>
      ) : null}
      {hints.length > 0 ? (
        <div className="hints">
          {hints.map((h) => (
            <button key={h.name} type="button" onClick={() => p.setDraft(`/${h.name} `)} translate="no">
              /{h.name} <span className="note">{h.about}</span>
            </button>
          ))}
        </div>
      ) : null}
      {error ? <div className="err">{error}</div> : null}
      <footer>
        <textarea
          aria-label={t('Mesaj')}
          placeholder={p.view.running ? t('Çalışan göreve mesaj ekle… (Enter)') : t('Kor\'a yaz… (Enter gönderir, / komutlar)')}
          value={p.draft}
          onChange={(e) => p.setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              send()
            }
          }}
        />
        {p.view.running ? (
          <button type="button" className="icon" aria-label={t('Durdur')} title={t('Durdur')} onClick={stop} data-kor-cmd="client.abort">
            <Square size={16} />
          </button>
        ) : null}
        <button type="button" className="icon primary" aria-label={t('Gönder')} title={t('Gönder')} onClick={send} data-kor-cmd="client.chat">
          <Send size={16} />
        </button>
      </footer>
    </section>
  )
}
