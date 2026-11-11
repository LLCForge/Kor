// Kor yan paneli (§18.3): Kor'un üçüncü istemcisi. Oturumlar ve olaylar çekirdekten gelir (CLI ve masaüstüyle aynı
// olay kaynağı); buradan verilen komut her yerde anında görünür.
import { useEffect, useMemo, useState } from 'react'
import { Settings } from 'lucide-react'
import { call, client, on, type Status } from './bus.ts'
import { Chat, type Context, type Session } from './Chat.tsx'
import { empty, fromTranscript, reduce, type View } from './events.ts'
import { Hosting, detect, type PanelInfo } from './Hosting.tsx'
import { ContextTools, DocsReader, FormFiller, PageToCode } from './PageTools.tsx'
import { Project } from './Project.tsx'
import { Setup } from './Setup.tsx'
import { t } from './i18n.ts'

type Tab = 'chat' | 'page' | 'project' | 'panel'

const offline: Status = { mode: 'none', core: false, paired: false, needsPairing: false, error: '', canLaunch: false }

export function App() {
  const [status, setStatus] = useState<Status>(offline)
  const [showSetup, setShowSetup] = useState(false)
  const [tab, setTab] = useState<Tab>('chat')
  const [sessions, setSessions] = useState<Session[]>([])
  const [current, setCurrent] = useState('')
  const [views, setViews] = useState<Record<string, View>>({})
  const [context, setContext] = useState<Context | null>(null)
  const [draft, setDraft] = useState('')
  const [model, setModel] = useState('')
  const [turns, setTurns] = useState(0)
  const [hosting, setHosting] = useState<PanelInfo | null>(null)
  const [steps, setSteps] = useState<Array<{ at: number; label: string; shot: string }>>([])
  const [pending, setPending] = useState<{ selection: string; url: string; title: string } | null>(null)
  const [notice, setNotice] = useState('')

  const loadSessions = () =>
    void client<Array<{ id: string; title: string }>>('sessions').then((s) => setSessions(s.map((x) => ({ id: x.id, title: x.title }))), () => undefined)

  const refreshPage = () => {
    void detect().then(setHosting)
    void call<{ kind: string; selection: string; url: string; title: string } | null>('pending').then((p) => {
      if (p) {
        setPending(p)
        setTab('page')
      }
    })
  }

  useEffect(() => {
    const off = on((m) => {
      if (m.kind === 'status') setStatus(m.status as Status)
      else if (m.kind === 'event') {
        const ev = (m.event ?? {}) as Record<string, unknown>
        const id = typeof ev.sessionId === 'string' ? ev.sessionId : ''
        if (!id) return
        setViews((all) => ({ ...all, [id]: reduce(all[id] ?? empty, ev) }))
        if (ev.type === 'done') {
          setTurns((n) => n + 1)
          loadSessions()
        }
      } else if (m.kind === 'tabChanged') refreshPage()
      else if (m.kind === 'pending') refreshPage()
      else if (m.kind === 'step') setSteps((s) => [...s, m.step as { at: number; label: string; shot: string }])
    })
    void call<Status>('status').then(setStatus, () => undefined)
    void call<Array<{ at: number; label: string; shot: string }>>('steps').then(setSteps, () => undefined)
    refreshPage()
    return off
  }, [])

  useEffect(() => {
    if (!status.paired) return
    loadSessions()
    // Çalışan görevler ve bekleyen onaylar baştan kurulur (CLI/masaüstünde başlamış olabilir).
    void client<Array<{ sessionId: string; events: Array<Record<string, unknown>> }>>('live').then((live) => {
      const next: Record<string, View> = {}
      for (const l of live) next[l.sessionId] = l.events.reduce(reduce, empty)
      setViews((all) => ({ ...all, ...next }))
      if (live[0]) setCurrent((c) => c || live[0].sessionId)
    }, () => undefined)
    void client<{ model: string | null }>('status').then((s) => setModel(s.model ?? ''), () => undefined)
  }, [status.paired])

  const select = (id: string) => {
    setCurrent(id)
    if (id && !views[id]?.running) {
      void client('transcript', { sessionId: id }).then((tr) => setViews((all) => ({ ...all, [id]: { ...empty, items: fromTranscript(tr) } })), () => undefined)
    }
  }
  const view = useMemo(() => views[current] ?? empty, [views, current])

  if (!status.paired || showSetup) {
    return (
      <div className="app">
        <Header status={status} onGear={() => setShowSetup(!showSetup)} />
        {!status.core ? <Offline canLaunch={status.canLaunch} /> : null}
        <Setup status={status} />
      </div>
    )
  }

  return (
    <div className="app">
      <Header status={status} onGear={() => setShowSetup(true)} />
      <nav className="tabs" role="tablist">
        {(['chat', 'page', 'project', ...(hosting ? ['panel'] : [])] as Tab[]).map((x) => (
          <button key={x} type="button" role="tab" aria-selected={tab === x} className={tab === x ? 'on' : ''} onClick={() => setTab(x)}>
            {x === 'chat' ? t('Sohbet') : x === 'page' ? t('Sayfa') : x === 'project' ? t('Proje') : t('Panel')}
          </button>
        ))}
      </nav>
      {notice ? <p className="note" role="status">{notice}</p> : null}
      {tab === 'chat' ? (
        <Chat
          sessions={sessions}
          current={current}
          view={view}
          context={context}
          draft={draft}
          setDraft={setDraft}
          onSelect={select}
          onSent={(id, text) => {
            setCurrent(id)
            setViews((all) => ({ ...all, [id]: { ...(all[id] ?? empty), items: [...(all[id] ?? empty).items, { kind: 'user', text }] } }))
          }}
          onAnswered={(v) => setViews((all) => ({ ...all, [current]: v }))}
          clearContext={() => setContext(null)}
        />
      ) : null}
      {tab === 'page' ? (
        <div className="scroll">
          <ContextTools
            setContext={(c) => {
              setContext(c)
              setTab('chat')
            }}
            onAdopt={setNotice}
          />
          <DocsReader
            pending={pending}
            onApplied={(id) => {
              setCurrent(id)
              setTab('chat')
            }}
          />
          <FormFiller />
          <PageToCode />
        </div>
      ) : null}
      {tab === 'project' ? (
        <div className="scroll">
          <Project
            sessionId={current}
            turns={turns}
            model={model}
            setDraft={(d) => {
              setDraft(d)
              setTab('chat')
            }}
          />
        </div>
      ) : null}
      {tab === 'panel' && hosting ? (
        <div className="scroll">
          <Hosting info={hosting} steps={steps} />
        </div>
      ) : null}
    </div>
  )
}

function Header({ status, onGear }: { status: Status; onGear: () => void }) {
  return (
    <header>
      <img className="logo" src="icons/48.png" alt="" />
      <h1>Kor</h1>
      <span className={`pill${status.paired ? ' on' : ''}`} role="img" aria-label={status.paired ? t('bağlı') : t('bağlı değil')} title={status.paired ? t('bağlı') : t('bağlı değil')} />
      <button type="button" className="icon" aria-label={t('Ayarlar')} title={t('Ayarlar')} onClick={onGear}>
        <Settings size={16} />
      </button>
    </header>
  )
}

function Offline({ canLaunch }: { canLaunch: boolean }) {
  return (
    <div className="offline" role="status">
      <p>{t('Kor çalışmıyor — Kor masaüstünü açın ya da bir terminalde Kor\'u başlatın.')}</p>
      {canLaunch ? (
        <button type="button" className="primary" onClick={() => void call('launch')}>
          {t('Kor\'u başlat')}
        </button>
      ) : (
        <p className="note">{t('Tek tıkla başlatma için: kor chrome install')}</p>
      )}
    </div>
  )
}
