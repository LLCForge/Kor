// §27.1 tepsi açılır penceresi: çalışan görevler (ad, süre, fiil, durdur), onay bekleyenler (onayla/reddet,
// ayrıntı), hızlı görev kutusu (tek satır + proje seçici; Enter ile başlar, ana pencere açılmaz), son 3 proje,
// "Kor'u aç", "Güncelle" (varsa).
import { Check, ExternalLink, Square, X } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { api, errorMessage } from '../api.ts'
import { shellui, useShellState } from './bridge.ts'
import { SOURCE_LABEL, countdown, elapsed, etaLabel, folderName, updatePillLabel, usd } from './state.ts'
import { displayPath } from '../display-path.ts'

export function TrayPopup() {
  const { data, error, isLoading } = useShellState()
  const [prompt, setPrompt] = useState('')
  const [root, setRoot] = useState('')
  const [note, setNote] = useState<string | null>(null)
  const recent = data?.recent ?? []
  const project = root || recent[0] || ''

  const run = async (fn: () => Promise<unknown>) => {
    setNote(null)
    try {
      await fn()
    } catch (err) {
      setNote(errorMessage(err))
    }
  }
  const submit = (e: FormEvent) => {
    e.preventDefault()
    const text = prompt.trim()
    if (!text) return
    void run(async () => {
      await shellui.quickTask(project, text)
      setPrompt('')
      setNote('Görev başladı.')
    })
  }

  if (isLoading) return <main className="ksh-popup"><p className="ksh-muted" role="status">Yükleniyor…</p></main>
  if (error || !data) {
    return (
      <main className="ksh-popup">
        <p className="error-text" role="alert">
          Durum alınamadı: {errorMessage(error)}
        </p>
      </main>
    )
  }
  const o = data.overview
  const now = Date.now()
  return (
    <main className="ksh-popup" data-kor-cmd="shellui.state" data-kor-event="kor:shellui/changed">
      <section aria-label="Çalışan görevler">
        <h2>Çalışanlar</h2>
        {o.sessions.length === 0 && o.daemon.jobs.length === 0 && o.remote.length === 0 && <p className="ksh-muted">Çalışan görev yok.</p>}
        <ul className="ksh-list">
          {o.sessions.map((s) => (
            <li key={s.id} className="ksh-row">
              <span className={s.waiting ? 'ksh-pip-dot warn' : 'ksh-pip-spinner'} aria-hidden="true" />
              <span className="ksh-grow" translate="no" title={displayPath(s.root)}>
                {s.title || folderName(s.root)}
              </span>
              <span className="ksh-muted">{s.waiting ? 'onay bekliyor' : s.verb}</span>
              <span className="ksh-muted">{elapsed(s.since, now)}</span>
              <span className="ksh-muted">{usd(s.costUsd, s.costEstimated)}</span>
              {s.eta && (
                <span className="ksh-muted" title={etaLabel(s.eta) ?? undefined}>
                  {countdown(s.eta, now) ?? etaLabel(s.eta)}
                </span>
              )}
              <button type="button" className="action-btn" aria-label="Sohbeti aç" title="Sohbeti aç" onClick={() => void shellui.openMain(s.id)} data-kor-cmd="shellui.openMain">
                <ExternalLink size={14} aria-hidden="true" />
              </button>
              <button type="button" className="action-btn" aria-label="Durdur" title="Durdur" onClick={() => void run(() => shellui.stop(s.id))} data-kor-cmd="shellui.stop">
                <Square size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
          {o.daemon.jobs.map((j) => (
            <li key={j.id} className="ksh-row">
              <span className="ksh-pip-spinner" aria-hidden="true" />
              <span className="ksh-grow" translate="no">
                {j.trigger}
              </span>
              <span className="ksh-chip">Daemon</span>
              <span className="ksh-muted">{j.status}</span>
              {j.started !== null && <span className="ksh-muted">{elapsed(j.started, now)}</span>}
              {j.eta && <span className="ksh-muted">{countdown(j.eta, now) ?? etaLabel(j.eta)}</span>}
            </li>
          ))}
          {o.remote.map((t) => (
            <li key={t.id} className="ksh-row">
              <span className="ksh-pip-spinner" aria-hidden="true" />
              <span className="ksh-grow" translate="no">
                {t.prompt}
              </span>
              <span className="ksh-chip">Uzak</span>
              <span className="ksh-muted">{elapsed(t.started, now)}</span>
            </li>
          ))}
        </ul>
      </section>
      {o.approvals.length > 0 && (
        <section aria-label="Onay bekleyenler">
          <h2>Onay bekleyenler</h2>
          <ul className="ksh-list">
            {o.approvals.map((a) => (
              <li key={a.id} className="ksh-row wrap">
                <span className={`ksh-risk ${a.risk}`}>{SOURCE_LABEL[a.source]}</span>
                <span className="ksh-grow" translate="no" title={a.summary}>
                  {a.tool}: {a.summary}
                </span>
                <button type="button" className="btn small" onClick={() => void run(() => shellui.deny(a.id))} data-kor-cmd="shellui.deny">
                  <X size={12} aria-hidden="true" /> Reddet
                </button>
                <button type="button" className="btn small primary" onClick={() => void run(() => shellui.approve(a.id))} data-kor-cmd="shellui.approve">
                  <Check size={12} aria-hidden="true" /> Onayla
                </button>
                {a.source === 'desktop' && (
                  <button type="button" className="btn small" onClick={() => void shellui.openMain(a.owner)} data-kor-cmd="shellui.openMain">
                    Ayrıntı
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
      <form className="ksh-quick" onSubmit={submit} aria-label="Hızlı görev">
        <input
          type="text"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Hızlı görev… (Enter ile başlar)"
          aria-label="Hızlı görev"
          maxLength={20000}
          data-kor-cmd="shellui.quickTask"
        />
        <select value={project} onChange={(e) => setRoot(e.target.value)} aria-label="Proje">
          {recent.length === 0 && <option value="">Varsayılan klasör</option>}
          {recent.map((r) => (
            <option key={r} value={r} title={r}>
              {folderName(r)}
            </option>
          ))}
        </select>
      </form>
      {note && <p className="ksh-muted" role="status">{note}</p>}
      {recent.length > 0 && (
        <section aria-label="Son projeler">
          <h2>Son projeler</h2>
          <ul className="ksh-list">
            {recent.map((r) => (
              <li key={r}>
                <button type="button" className="ksh-link" title={r} onClick={() => void run(() => shellui.openProject(r))} data-kor-cmd="shellui.openProject">
                  {folderName(r)}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <footer className="ksh-actions">
        <button type="button" className="btn small" onClick={() => void shellui.openMain(null)} data-kor-cmd="shellui.openMain">
          Kor'u aç
        </button>
        <button type="button" className="btn small" onClick={() => void shellui.pip()} data-kor-cmd="shellui.pip">
          Durum penceresi
        </button>
        {data.update && (
          <button type="button" className="ksh-pill" onClick={() => void run(() => api.g16UpdateInstall())} data-kor-cmd="g16UpdateInstall">
            {updatePillLabel(data.update)}
          </button>
        )}
      </footer>
    </main>
  )
}
