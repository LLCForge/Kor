// Ayarlar > Ajanlar > Takımlar (cdesktop 2 sütun): solda takımlar (ad + kaynak), sağda roller sırası ve talimatları.
// Kullanıcı takımları ~/.kor/teams.json'a yazılır: "Takım oluştur" diyaloğu ad, açıklama ve rol satırları alır.
import { ArrowRight, Pencil, Plus, Trash2, Users } from 'lucide-react'
import { useId, useMemo, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { AgentSummary } from '../../ipc-agents.ts'
import type { TeamRoleView, TeamView } from '../../ipc-teams.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { Modal } from '../NameDialog.tsx'

export const TEAMS_KEY = 'teams:list'
const SOURCE_LABEL: Record<TeamView['source'], string> = { builtin: 'yerleşik', user: 'kullanıcı', project: 'proje' }
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/
const EMPTY_ROLE: TeamRoleView = { role: '', agent: 'general-purpose', instructions: 'Görev: {task}\n\nÖnceki roller:\n{previous}\n\n' }

function TeamDialog({ initial, agents, existing, onSave, onClose }: { initial: TeamView | null; agents: readonly AgentSummary[]; existing: readonly TeamView[]; onSave: (team: Omit<TeamView, 'source'>) => Promise<void>; onClose: () => void }) {
  const titleId = useId()
  const fid = useId()
  const [name, setName] = useState(initial?.name ?? '')
  const [description, setDescription] = useState(initial?.description ?? '')
  const [roles, setRoles] = useState<TeamRoleView[]>(initial?.roles.map((r) => ({ ...r })) ?? [{ ...EMPTY_ROLE }])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nameOk = NAME_RE.test(name.trim())
  const duplicate = !initial && existing.some((t) => t.name.toLowerCase() === name.trim().toLowerCase() && t.source === 'user')
  const rolesOk = roles.length > 0 && roles.every((r) => r.role.trim() && r.agent.trim() && r.instructions.trim())
  const update = (i: number, patch: Partial<TeamRoleView>) => setRoles((rs) => rs.map((r, k) => (k === i ? { ...r, ...patch } : r)))
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!nameOk || duplicate || !description.trim() || !rolesOk) return
    setBusy(true)
    setError(null)
    try {
      await onSave({ name: name.trim(), description: description.trim(), roles: roles.map((r) => ({ role: r.role.trim(), agent: r.agent.trim(), instructions: r.instructions })) })
      onClose()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <form className="dialog-body cdx-team-form" onSubmit={(e) => void submit(e)}>
        <h2 id={titleId} className="dialog-title">{initial ? 'Takımı düzenle' : 'Takım oluştur'}</h2>
        <p className="dialog-desc">Roller sırayla çalışır; her rol önceki rollerin raporunu {'{previous}'}, görevi {'{task}'} ile alır.</p>
        <label htmlFor={`${fid}-n`}>Takım adı <span className="cdx-req">*</span></label>
        <input id={`${fid}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} disabled={!!initial} placeholder="ör. urun-ekibi" spellCheck={false} maxLength={40} aria-invalid={name !== '' && (!nameOk || duplicate)} />
        {name && !nameOk && <p className="error-text g9-small">Ad harf/rakamla başlamalı; yalnızca harf, rakam, _ ve -.</p>}
        {duplicate && <p className="error-text g9-small">Bu adda bir kullanıcı takımı zaten var.</p>}
        <label htmlFor={`${fid}-d`}>Açıklama <span className="cdx-req">*</span></label>
        <input id={`${fid}-d`} className="text-input" value={description} onChange={(e) => setDescription(e.target.value)} maxLength={300} placeholder="Takımın ne yaptığı (TEAMS listesinde görünür)" />
        <div className="cdx-team-roles-head">
          <span>Roller (sırayla)</span>
          <button type="button" className="btn small" onClick={() => setRoles((rs) => [...rs, { ...EMPTY_ROLE }])} disabled={roles.length >= 12}><Plus size={13} aria-hidden="true" /> Rol ekle</button>
        </div>
        {roles.map((r, i) => (
          <div key={i} className="cdx-team-role">
            <div className="cdx-team-role-row">
              <span className="cdx-team-role-no">{i + 1}</span>
              <input className="text-input" value={r.role} onChange={(e) => update(i, { role: e.target.value })} placeholder="rol adı (ör. planlayıcı)" maxLength={40} aria-label={`${i + 1}. rol adı`} />
              <select className="g7-select" value={r.agent} onChange={(e) => update(i, { agent: e.target.value })} aria-label={`${i + 1}. rol ajanı`}>
                {!agents.some((a) => a.name === r.agent) && <option value={r.agent}>{r.agent}</option>}
                {agents.map((a) => <option key={a.name} value={a.name}>{a.name}</option>)}
              </select>
              <button type="button" className="action-btn" onClick={() => setRoles((rs) => rs.filter((_x, k) => k !== i))} disabled={roles.length <= 1} aria-label="Rolü sil" title="Rolü sil"><Trash2 size={13} aria-hidden="true" /></button>
            </div>
            <textarea className="text-input dialog-textarea" rows={3} value={r.instructions} onChange={(e) => update(i, { instructions: e.target.value })} placeholder="Bu rolün talimatı ({task}, {previous})" aria-label={`${i + 1}. rol talimatı`} />
          </div>
        ))}
        {error && <p className="error-text" role="alert">{error}</p>}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose} disabled={busy}>İptal</button>
            <button type="submit" className="btn primary" disabled={busy || !nameOk || duplicate || !description.trim() || !rolesOk}>{initial ? 'Kaydet' : 'Oluştur'}</button>
          </div>
        </div>
      </form>
    </Modal>
  )
}

export function TeamsSection({ agents }: { agents: readonly AgentSummary[] }) {
  const { data, error, mutate } = useSWR(TEAMS_KEY, () => api.teams.list(), { revalidateOnFocus: false })
  const [selectedName, setSelectedName] = useState<string | null>(null)
  const [dialog, setDialog] = useState<{ initial: TeamView | null } | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const teams = data ?? []
  const selected = useMemo(() => teams.find((t) => t.name === selectedName) ?? teams[0] ?? null, [teams, selectedName])
  const userTeams = teams.filter((t) => t.source === 'user')
  const persist = async (next: Array<Omit<TeamView, 'source'>>) => {
    const saved = await api.teams.saveUser(next)
    await mutate(saved, { revalidate: false })
  }
  const strip = (t: TeamView): Omit<TeamView, 'source'> => ({ name: t.name, description: t.description, roles: t.roles })
  const remove = (team: TeamView) =>
    void confirmAction({ title: 'Takım silinsin mi?', message: `"${team.name}" kullanıcı takımı kaldırılacak.`, confirmLabel: 'Sil', danger: true }).then((ok) => {
      if (ok) persist(userTeams.filter((t) => t.name !== team.name).map(strip)).catch((err: unknown) => setMessage(errorMessage(err)))
    })

  if (error) return <p className="error-text">{errorMessage(error)}</p>
  return (
    <section className="g7-subsection cdx-teams">
      <div className="cdx-tags-head">
        <div>
          <h4 className="g7-h4"><Users size={14} aria-hidden="true" /> Takımlar</h4>
          <p className="g7-note">Rol tabanlı ajan ekipleri: roller sırayla alt ajan olarak çalışır. Sohbette <code>/takim &lt;takım&gt; &lt;görev&gt;</code> ya da modelin <code>run_team</code> aracı.</p>
        </div>
        <button type="button" className="btn small" onClick={() => setDialog({ initial: null })}><Plus size={14} aria-hidden="true" /> Takım oluştur</button>
      </div>
      {message && <p className="error-text" role="status">{message}</p>}
      {teams.length > 0 && (
        <div className="cdx-agents">
          <div className="cdx-agents-col">
            <div className="cdx-agents-head">Takım</div>
            <ul className="cdx-agents-list" role="listbox" aria-label="Takımlar">
              {teams.map((t) => {
                const active = selected?.name === t.name
                return (
                  <li key={t.name}>
                    <button type="button" role="option" aria-selected={active} className={`cdx-agents-item${active ? ' active' : ''}`} onClick={() => setSelectedName(t.name)}>
                      <Users size={14} aria-hidden="true" />
                      <span className="truncate">{t.name}</span>
                      <span className="badge neutral cdx-agents-src">{SOURCE_LABEL[t.source]}</span>
                    </button>
                  </li>
                )
              })}
            </ul>
          </div>
          <div className="cdx-agents-col cdx-agents-detail">
            <div className="cdx-agents-head">
              Roller
              {selected?.source === 'user' && (
                <span className="cdx-team-actions">
                  <button type="button" className="action-btn" onClick={() => setDialog({ initial: selected })} aria-label="Takımı düzenle" title="Düzenle"><Pencil size={13} aria-hidden="true" /></button>
                  <button type="button" className="action-btn" onClick={() => remove(selected)} aria-label="Takımı sil" title="Sil"><Trash2 size={13} aria-hidden="true" /></button>
                </span>
              )}
            </div>
            {selected && (
              <div className="cdx-agent-form">
                <p className="g7-note cdx-team-desc">{selected.description}</p>
                <ol className="cdx-team-flow" aria-label="Rol sırası">
                  {selected.roles.map((r, i) => (
                    <li key={i} className="cdx-team-step">
                      <div className="cdx-team-step-head">
                        <span className="cdx-team-role-no">{i + 1}</span>
                        <strong>{r.role}</strong>
                        <ArrowRight size={12} aria-hidden="true" />
                        <code>{r.agent}</code>
                      </div>
                      <pre className="cdx-team-instr">{r.instructions}</pre>
                    </li>
                  ))}
                </ol>
              </div>
            )}
          </div>
        </div>
      )}
      {dialog && (
        <TeamDialog
          initial={dialog.initial}
          agents={agents}
          existing={teams}
          onClose={() => setDialog(null)}
          onSave={(team) => persist(dialog.initial ? userTeams.map((t) => (t.name === team.name ? team : strip(t))) : [...userTeams.map(strip), team])}
        />
      )}
    </section>
  )
}
