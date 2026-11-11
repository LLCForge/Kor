// decisions.md girdileri (§6.1), yeni karar ekleme (dosyanın sonuna eklenir, kullanıcı metni korunur) ve
// isteğe bağlı git notları (`kor memory notes push|pull`).
import { Download, Plus, Upload } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { memoryApi, projmemApi } from './api.ts'
import { parseRefs } from './shape.ts'

export function DecisionsTab({ project, gitNotes, onChanged }: { project: string; gitNotes: boolean; onChanged: () => void }) {
  const id = useId()
  const { data, error, mutate } = useSWR(['memory:decisions', project], () => memoryApi.decisions())
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [refs, setRefs] = useState('')
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)

  const run = (p: Promise<string>) => {
    setBusy(true)
    setNote(null)
    p.then((text) => setNote({ text, ok: true }))
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
      .finally(() => setBusy(false))
  }

  const add = (e: FormEvent) => {
    e.preventDefault()
    run(
      projmemApi.addDecision(title.trim(), body, parseRefs(refs)).then((r) => {
        void mutate((d) => (d ? { ...d, decisions: r.decisions } : d), { revalidate: false })
        setTitle('')
        setBody('')
        setRefs('')
        onChanged()
        return 'Karar eklendi.'
      }),
    )
  }

  return (
    <div className="g14-section">
      <section className="g14-section" aria-labelledby={`${id}-list`} data-kor-cmd="memory.decisions">
        <h3 className="g7-h4" id={`${id}-list`}>Kararlar</h3>
        {error ? (
          <p className="error-text" role="alert">{errorMessage(error)}</p>
        ) : !data ? (
          <p className="muted">Yükleniyor...</p>
        ) : data.decisions.length === 0 ? (
          <p className="g7-note">Henüz karar yok. Aşağıdan ekleyebilir ya da decisions.md'ye `## Başlık` ile yazabilirsin.</p>
        ) : (
          <ol className="core-list kmem-decisions">
            {[...data.decisions].reverse().map((d, i) => (
              <li key={`${d.date}:${d.title}:${i}`} className="kmem-card">
                <div className="kmem-row">
                  <strong translate="no">{d.title}</strong>
                  {d.date && <span className="muted g7-mono">{d.date}</span>}
                </div>
                {d.body && <p className="kmem-body" translate="no">{d.body}</p>}
                {d.refs.length > 0 && (
                  <p className="g7-note">
                    İlgili: <span className="g7-mono" translate="no">{d.refs.join(', ')}</span>
                  </p>
                )}
              </li>
            ))}
          </ol>
        )}
      </section>

      <form className="g14-section g7-subsection" onSubmit={add} aria-labelledby={`${id}-add`} data-kor-cmd="projmem.addDecision">
        <h3 className="g7-h4" id={`${id}-add`}>Karar ekle</h3>
        <label className="kmem-field">
          <span>Başlık</span>
          <input className="text-input" value={title} onChange={(e) => setTitle(e.target.value)} maxLength={200} required translate="no" />
        </label>
        <label className="kmem-field">
          <span>Gerekçe</span>
          <textarea className="text-input kmem-textarea" value={body} onChange={(e) => setBody(e.target.value)} maxLength={20000} rows={4} />
        </label>
        <label className="kmem-field">
          <span>İlgili dosya ya da semboller (virgülle)</span>
          <input className="text-input" value={refs} onChange={(e) => setRefs(e.target.value)} placeholder="src/auth.rs, Auth::login" translate="no" />
        </label>
        <div className="kmem-row">
          <button type="submit" className="btn small primary" disabled={busy || !title.trim()}>
            <Plus size={14} aria-hidden="true" /> Ekle
          </button>
        </div>
      </form>

      <section className="g14-section g7-subsection" aria-labelledby={`${id}-notes`}>
        <h3 className="g7-h4" id={`${id}-notes`}>Git notları</h3>
        {gitNotes ? (
          <>
            <p className="g7-note">
              decisions.md, deponun kök commit'ine <code>refs/notes/kor</code> notu olarak yazılır. Kor ağa çıkmaz; uzak depoya göndermek için
              <code> git push origin refs/notes/kor</code>.
            </p>
            <div className="kmem-row">
              <button
                type="button"
                className="btn small"
                disabled={busy}
                data-kor-cmd="memory.notesPush"
                onClick={() => run(memoryApi.notesPush().then((r) => `Not yazıldı (${r.commit.slice(0, 10)}).`))}
              >
                <Upload size={14} aria-hidden="true" /> Notlara yaz
              </button>
              <button
                type="button"
                className="btn small"
                disabled={busy}
                data-kor-cmd="memory.notesPull"
                onClick={() =>
                  run(
                    memoryApi.notesPull().then((r) => {
                      void mutate()
                      onChanged()
                      return `${r.added} karar eklendi.`
                    }),
                  )
                }
              >
                <Download size={14} aria-hidden="true" /> Notlardan al
              </button>
            </div>
          </>
        ) : (
          <p className="g7-note">
            Kapalı. Açmak için: <code>kor config set memory.git_notes true</code>
          </p>
        )}
      </section>
      {note && (
        <p className={note.ok ? 'g7-note' : 'error-text'} role={note.ok ? 'status' : 'alert'}>
          {note.text}
        </p>
      )}
    </div>
  )
}
