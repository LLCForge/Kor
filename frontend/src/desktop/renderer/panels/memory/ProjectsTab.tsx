// Çoklu repo çalışma alanları (`kor workspace`, §31.3) ve bu projeye bağlı projeler (`kor context add|remove`, §6.1).
import { FolderPlus, Plus, X } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import type { KeyedMutator } from 'swr'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { memoryApi, projmemApi } from './api.ts'

interface Props {
  workspaces: { folders: string[] } | undefined
  linked: { projects: string[] } | undefined
  error: unknown
  mutateWorkspaces: KeyedMutator<{ folders: string[] }>
  mutateLinked: KeyedMutator<{ projects: string[] }>
}

export function ProjectsTab({ workspaces, linked, error, mutateWorkspaces, mutateLinked }: Props) {
  const id = useId()
  const [pattern, setPattern] = useState('')
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)

  const run = async (work: () => Promise<string | null>) => {
    setBusy(true)
    setNote(null)
    try {
      const text = await work()
      if (text) setNote({ text, ok: true })
    } catch (e) {
      setNote({ text: errorMessage(e), ok: false })
    } finally {
      setBusy(false)
    }
  }

  const addWorkspace = (p: string) =>
    run(async () => {
      const r = await memoryApi.workspaceAdd(p)
      await mutateWorkspaces()
      return `${r.added.length} klasör eklendi.`
    })

  const submitPattern = (e: FormEvent) => {
    e.preventDefault()
    if (pattern.trim()) void addWorkspace(pattern.trim()).then(() => setPattern(''))
  }

  const pick = () => projmemApi.pickFolder()

  const removeWorkspace = async (path: string) => {
    if (!(await confirmAction({ title: 'Çalışma alanından çıkarılsın mı?', message: `${path} çoklu repo görünümünden çıkarılacak. Klasör silinmez.`, confirmLabel: 'Çıkar', danger: true }))) return
    void run(async () => {
      await memoryApi.workspaceRemove(path)
      await mutateWorkspaces()
      return null
    })
  }

  const unlink = async (path: string) => {
    if (!(await confirmAction({ title: 'Bağlantı kaldırılsın mı?', message: `${path} bu projenin bağlamından çıkarılacak. Klasör silinmez.`, confirmLabel: 'Kaldır', danger: true }))) return
    void run(async () => {
      const r = await memoryApi.unlink(path)
      void mutateLinked({ projects: r.projects }, { revalidate: false })
      return null
    })
  }

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  return (
    <div className="g14-section">
      <section className="g14-section" aria-labelledby={`${id}-ws`} data-kor-cmd="memory.workspaces">
        <h3 className="g7-h4" id={`${id}-ws`}>Çalışma alanları</h3>
        <p className="g7-note">Çoklu repo görünümü: buradaki repolar bilgi tabanında ve grafta birlikte düşünülür.</p>
        {!workspaces ? (
          <p className="muted">Yükleniyor...</p>
        ) : workspaces.folders.length === 0 ? (
          <p className="g7-note">Henüz klasör yok.</p>
        ) : (
          <ul className="core-list">
            {workspaces.folders.map((f) => (
              <li key={f} className="core-row">
                <code className="core-row-main" translate="no">{f}</code>
                <button type="button" className="action-btn" aria-label="Çıkar" title="Çalışma alanından çıkar" onClick={() => void removeWorkspace(f)} data-kor-cmd="memory.workspaceRemove">
                  <X size={14} aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <form className="kmem-row kmem-wrap" onSubmit={submitPattern} data-kor-cmd="memory.workspaceAdd">
          <label className="kmem-grow">
            <span className="sr-only">Klasör ya da desen</span>
            <input className="text-input" value={pattern} onChange={(e) => setPattern(e.target.value)} placeholder="~/projeler/*" maxLength={1000} translate="no" />
          </label>
          <button type="submit" className="btn small" disabled={busy || !pattern.trim()}>
            <Plus size={14} aria-hidden="true" /> Ekle
          </button>
          <button
            type="button"
            className="btn small"
            disabled={busy}
            data-kor-cmd="projmem.pickFolder"
            onClick={() =>
              void pick()
                .then((dir) => (dir ? addWorkspace(dir) : undefined))
                .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
            }
          >
            <FolderPlus size={14} aria-hidden="true" /> Klasör seç
          </button>
        </form>
      </section>

      <section className="g14-section g7-subsection" aria-labelledby={`${id}-ln`} data-kor-cmd="memory.projects">
        <h3 className="g7-h4" id={`${id}-ln`}>Bağlı projeler</h3>
        <p className="g7-note">Bu projeyle aynı bağlamda açılan projeler; "X'teki auth'u buraya uyarla" gibi görevler iki grafı birlikte kullanır.</p>
        {!linked ? (
          <p className="muted">Yükleniyor...</p>
        ) : linked.projects.length === 0 ? (
          <p className="g7-note">Bağlı proje yok.</p>
        ) : (
          <ul className="core-list">
            {linked.projects.map((p) => (
              <li key={p} className="core-row">
                <code className="core-row-main" translate="no">{p}</code>
                <button type="button" className="action-btn" aria-label="Bağlantıyı kaldır" title="Bağlantıyı kaldır" onClick={() => void unlink(p)} data-kor-cmd="memory.unlink">
                  <X size={14} aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="kmem-row">
          <button
            type="button"
            className="btn small"
            disabled={busy}
            data-kor-cmd="memory.link"
            onClick={() =>
              void run(async () => {
                const dir = await pick()
                if (!dir) return null
                const r = await memoryApi.link(dir)
                void mutateLinked({ projects: r.projects }, { revalidate: false })
                return r.added ? 'Proje bağlandı.' : 'Bu proje zaten bağlı.'
              })
            }
          >
            <FolderPlus size={14} aria-hidden="true" /> Proje bağla
          </button>
        </div>
      </section>
      {note && (
        <p className={note.ok ? 'g7-note' : 'error-text'} role={note.ok ? 'status' : 'alert'}>
          {note.text}
        </p>
      )}
    </div>
  )
}
