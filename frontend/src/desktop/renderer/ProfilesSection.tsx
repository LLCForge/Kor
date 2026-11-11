// [cdx] Ayarlar > Genel > Profiller (cdesktop Agent Configuration Profiles): tablo + ekle/düzenle/sil diyaloğu.
// Yerleşik profiller (Varsayılan / Onaylı / Plan / Otomatik) salt okunur listelenir; özel profiller kullanıcı ayarına yazılır.
import { Pencil, Plus, Trash2, UserCircle2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { PermissionMode } from '../../core/permissions.ts'
import { REASONING_EFFORTS } from '../../core/reasoning.ts'
import type { AgentProfile } from '../g7-types.ts'
import { api, errorMessage } from './api.ts'
import { confirmAction } from './ConfirmDialog.tsx'
import { MODES } from './ModeMenu.tsx'
import { Modal } from './NameDialog.tsx'
import { BUILTIN_PROFILES, describeProfile } from './ProfileMenu.tsx'
import { useCatalog } from './g7-bus.ts'

const SETTINGS_KEY = 'g7:settings'

function ProfileDialog({ initial, existing, onSave, onClose }: { initial: AgentProfile | null; existing: readonly string[]; onSave: (p: AgentProfile) => Promise<void>; onClose: () => void }) {
  const titleId = useId()
  const fid = useId()
  const catalog = useCatalog(true)
  const [name, setName] = useState(initial?.name ?? '')
  const [mode, setMode] = useState<PermissionMode | ''>(initial?.mode ?? '')
  const [model, setModel] = useState(initial?.model ?? '')
  const [effort, setEffort] = useState<AgentProfile['effort'] | ''>(initial?.effort ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const trimmed = name.trim()
  const duplicate = trimmed !== (initial?.name ?? '') && existing.some((n) => n.toLowerCase() === trimmed.toLowerCase())
  const ok = trimmed.length > 0 && trimmed.length <= 40 && !duplicate
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!ok) return
    setBusy(true)
    setError(null)
    try {
      const p: AgentProfile = { name: trimmed }
      if (mode) p.mode = mode
      if (model.trim()) p.model = model.trim()
      if (effort) p.effort = effort
      await onSave(p)
      onClose()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <form className="dialog-body cdx-tag-form" onSubmit={(e) => void submit(e)}>
        <h2 id={titleId} className="dialog-title">{initial ? 'Profili düzenle' : 'Profil oluştur'}</h2>
        <label htmlFor={`${fid}-n`}>Profil adı <span className="cdx-req">*</span></label>
        <input id={`${fid}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="ör. Hızlı inceleme" spellCheck={false} autoFocus maxLength={40} aria-invalid={duplicate} />
        {duplicate && <p className="error-text g9-small">Bu adda bir profil zaten var.</p>}
        <label htmlFor={`${fid}-m`}>İzin modu</label>
        <select id={`${fid}-m`} className="g7-select" value={mode} onChange={(e) => setMode(e.target.value as PermissionMode | '')}>
          <option value="">Değiştirme</option>
          {MODES.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
        </select>
        <label htmlFor={`${fid}-mo`}>Model</label>
        <input id={`${fid}-mo`} className="text-input" list={`${fid}-models`} value={model} onChange={(e) => setModel(e.target.value)} placeholder="Değiştirme (boş) ya da model kimliği" spellCheck={false} />
        <datalist id={`${fid}-models`}>
          {(catalog.data?.models ?? []).map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
        </datalist>
        <label htmlFor={`${fid}-e`}>Akıl yürütme düzeyi</label>
        <select id={`${fid}-e`} className="g7-select" value={effort ?? ''} onChange={(e) => setEffort(e.target.value as AgentProfile['effort'] | '')}>
          <option value="">Değiştirme</option>
          {REASONING_EFFORTS.map((lv) => <option key={lv} value={lv}>{lv}</option>)}
        </select>
        {error && <p className="error-text" role="alert">{error}</p>}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose} disabled={busy}>İptal</button>
            <button type="submit" className="btn primary" disabled={busy || !ok}>{initial ? 'Kaydet' : 'Oluştur'}</button>
          </div>
        </div>
      </form>
    </Modal>
  )
}

export function ProfilesSection() {
  const settings = useSWR(SETTINGS_KEY, () => api.g7GetSettings())
  const [dialog, setDialog] = useState<{ initial: AgentProfile | null } | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const custom = settings.data?.profiles ?? []
  const names = [...BUILTIN_PROFILES.map((p) => p.name), ...custom.map((p) => p.name)]
  const persist = async (next: AgentProfile[]) => {
    await api.g7SetProfiles(next)
    await settings.mutate()
  }
  const remove = (p: AgentProfile) =>
    void confirmAction({ title: 'Profil silinsin mi?', message: `"${p.name}" profili kaldırılacak.`, confirmLabel: 'Sil', danger: true }).then((ok) => {
      if (ok) persist(custom.filter((c) => c.name !== p.name)).catch((err: unknown) => setMessage(errorMessage(err)))
    })
  return (
    <section className="g7-subsection cdx-tags">
      <div className="cdx-tags-head">
        <div>
          <h4 className="g7-h4">Profiller</h4>
          <p className="g7-note">Mesaj kutusundaki profil seçicisinden tek tıkla uygulanan izin modu + model + akıl yürütme düzeyi paketleri.</p>
        </div>
        <button type="button" className="btn small" onClick={() => setDialog({ initial: null })}>
          <Plus size={14} aria-hidden="true" /> Profil ekle
        </button>
      </div>
      {message && <p className="error-text" role="status">{message}</p>}
      <table className="cdx-tags-table">
        <thead>
          <tr><th>Profil</th><th>Ayarlar</th><th className="cdx-tags-actions">İşlemler</th></tr>
        </thead>
        <tbody>
          {BUILTIN_PROFILES.map((p) => (
            <tr key={p.name}>
              <td><UserCircle2 size={13} aria-hidden="true" /> {p.name} <span className="badge neutral">Yerleşik</span></td>
              <td className="cdx-tags-content">{p.desc}</td>
              <td className="cdx-tags-actions" />
            </tr>
          ))}
          {custom.map((p) => (
            <tr key={p.name}>
              <td><UserCircle2 size={13} aria-hidden="true" /> {p.name}</td>
              <td className="cdx-tags-content">{describeProfile(p, 'default') || 'Hiçbir şeyi değiştirmez'}</td>
              <td className="cdx-tags-actions">
                <button type="button" className="action-btn" onClick={() => setDialog({ initial: p })} aria-label={`${p.name} düzenle`} title="Düzenle"><Pencil size={13} aria-hidden="true" /></button>
                <button type="button" className="action-btn" onClick={() => remove(p)} aria-label={`${p.name} sil`} title="Sil"><Trash2 size={13} aria-hidden="true" /></button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {dialog && (
        <ProfileDialog
          initial={dialog.initial}
          existing={names}
          onClose={() => setDialog(null)}
          onSave={(p) => persist(dialog.initial ? custom.map((c) => (c.name === dialog.initial!.name ? p : c)) : [...custom, p])}
        />
      )}
    </section>
  )
}
