// SEO ajan takımı (§29.15): Baş Denetçi taramayı bölüştürür, uzman ajanlar paralel çalışır, bulgular tek rapora
// ve tek düzeltme listesine birleşir. Ajan başına kart `kor:seo/agent` olaylarından. CLI: kor seo agents <site>.
import { CalendarClock, Users } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import type { SeoDepth, SeoTeamReport } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { seoApi } from './api.ts'
import { Findings } from './Findings.tsx'
import { AGENT_LABEL, SEVERITY_LABEL, foldAgent, type AgentCard } from './format.ts'

const PICKABLE = ['technical', 'performance', 'content', 'structured', 'local', 'security', 'competitor', 'log'] as const

export function AgentsSection() {
  const uid = useId()
  const [site, setSite] = useState('')
  const [depth, setDepth] = useState<SeoDepth>('quick')
  const [picked, setPicked] = useState<string[]>([])
  const [log, setLog] = useState('')
  const [verifyDns, setVerifyDns] = useState(false)
  const [useModel, setUseModel] = useState(true)
  const [fix, setFix] = useState(false)
  const [notify, setNotify] = useState(false)
  const [busy, setBusy] = useState(false)
  const [cards, setCards] = useState<AgentCard[]>([])
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [team, setTeam] = useState<SeoTeamReport | null>(null)

  useEffect(() => seoApi.onAgent((e) => setCards((c) => foldAgent(c, e))), [])

  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]))
  const submit = (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    setCards([])
    setTeam(null)
    seoApi
      .agents(site.trim(), depth, picked, log.trim() || null, verifyDns, useModel, fix, notify)
      .then(setTeam)
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false))
  }
  const schedule = (every: 'daily' | 'weekly') => {
    setNote(null)
    // Nöbet modu: rutin `kor seo agents <site> --depth quick` çalıştırır ve öncekiyle farkı bildirir.
    seoApi
      .agentsSchedule(site.trim(), every)
      .then(() => setNote(every === 'daily' ? 'Günlük nöbet rutini eklendi.' : 'Haftalık nöbet rutini eklendi.'))
      .catch((err: unknown) => setNote(errorMessage(err)))
  }

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h3 id={`${uid}-h`} className="kg-card-title ks-title">
        <Users size={16} aria-hidden="true" />
        SEO ajan takımı
      </h3>
      <form className="kp-form" onSubmit={submit}>
        <label className="kg-field kp-grow">
          <span>Site</span>
          <input value={site} onChange={(e) => setSite(e.target.value)} placeholder="ornek.com" spellCheck={false} maxLength={300} translate="no" required />
        </label>
        <label className="kg-field">
          <span>Derinlik</span>
          <select value={depth} onChange={(e) => setDepth(e.target.value as SeoDepth)}>
            <option value="quick">Hızlı</option>
            <option value="full">Tam</option>
            <option value="forensic">Adli</option>
          </select>
        </label>
        <label className="kg-field kp-grow">
          <span>Erişim logu (Log ajanı için, isteğe bağlı)</span>
          <input value={log} onChange={(e) => setLog(e.target.value)} placeholder="/var/log/nginx/access.log" spellCheck={false} maxLength={500} translate="no" />
        </label>
        <button type="submit" className="btn small" disabled={busy || !site.trim()} data-kor-cmd="seo.agents">
          <Users size={14} aria-hidden="true" />
          {busy ? 'Ajanlar çalışıyor…' : 'Takımı çalıştır'}
        </button>
      </form>
      <fieldset className="kseo-tabs">
        <legend className="sr-only">Ajanlar (boş: hepsi)</legend>
        {PICKABLE.map((id) => (
          <label key={id} className="kseo-check">
            <input type="checkbox" checked={picked.includes(id)} onChange={() => toggle(id)} />
            {AGENT_LABEL[id]}
          </label>
        ))}
      </fieldset>
      <div className="kseo-tabs">
        <label className="kseo-check"><input type="checkbox" checked={useModel} onChange={(e) => setUseModel(e.target.checked)} />Raporcu özeti (model)</label>
        <label className="kseo-check"><input type="checkbox" checked={verifyDns} onChange={(e) => setVerifyDns(e.target.checked)} />Botları ters DNS ile doğrula</label>
        <label className="kseo-check"><input type="checkbox" checked={fix} onChange={(e) => setFix(e.target.checked)} />Düzeltme listesini .kor/seo/fixes'e yaz</label>
        <label className="kseo-check"><input type="checkbox" checked={notify} onChange={(e) => setNotify(e.target.checked)} />Yeni bulguları bildir</label>
        <button type="button" className="btn small ghost" disabled={!site.trim()} onClick={() => schedule('weekly')} data-kor-cmd="seo.agentsSchedule">
          <CalendarClock size={14} aria-hidden="true" />
          Haftalık nöbet
        </button>
      </div>
      {note && <p className="muted kp-small" role="status">{note}</p>}
      {cards.length > 0 && (
        <ul className="kseo-cards" data-kor-event="kor:seo/agent" aria-label="Ajanlar">
          {cards.map((c) => (
            <li key={c.id} className="kseo-card" data-state={c.state}>
              <strong>{AGENT_LABEL[c.id] ?? c.id}</strong>
              <span className="muted">{c.state === 'running' ? 'çalışıyor' : c.state === 'skipped' ? 'atlandı' : `${c.findings} bulgu`}</span>
              {c.detail && <span className="muted kp-loc" translate="no">{c.detail}</span>}
            </li>
          ))}
        </ul>
      )}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
      {team && (
        <>
          {team.summary && (
            <div>
              <h4 className="kp-h">Özet (Raporcu)</h4>
              <p className="kp-pre kp-small" translate="no">{team.summary}</p>
            </div>
          )}
          <h4 className="kp-h">Öncelikler (etki × zorluk)</h4>
          <div className="kp-table-wrap">
            <table className="kp-table">
              <thead>
                <tr>
                  <th scope="col">Kural</th>
                  <th scope="col">Ajan</th>
                  <th scope="col">Önem</th>
                  <th scope="col">Sayfa</th>
                  <th scope="col">Zorluk</th>
                  <th scope="col">Skor</th>
                </tr>
              </thead>
              <tbody>
                {team.priorities.slice(0, 25).map((p) => (
                  <tr key={p.rule}>
                    <td className="kp-fn" translate="no">{p.rule}</td>
                    <td>{AGENT_LABEL[p.agent] ?? p.agent}</td>
                    <td>{SEVERITY_LABEL[p.severity]}</td>
                    <td className="kp-num">{p.pages}</td>
                    <td className="kp-num">{p.effort}</td>
                    <td className="kp-num">{p.score.toFixed(1)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {team.conflicts.length > 0 && (
            <ul className="kp-notes" aria-label="Baş Denetçi kararları">
              {team.conflicts.map(([u, n]) => <li key={u} translate="no">{`${u}: ${n}`}</li>)}
            </ul>
          )}
          {team.diff && <p className="kp-small">{`Önceki takım denetiminden beri ${team.diff.new.length} yeni, ${team.diff.fixed.length} düzelmiş bulgu.`}</p>}
          <Findings report={team.report} title={`SEO ajan takımı: ${site}`} file="seo-agents.html" />
        </>
      )}
    </section>
  )
}
