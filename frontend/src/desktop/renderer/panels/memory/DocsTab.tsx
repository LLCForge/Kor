// Proje hafıza belgeleri (§6.1): architecture/decisions/patterns/hotspots düzenlenir, oturum özetleri salt okunur.
// Kor'un işaretli bölümleri gösterilir; kayıt, belge okunduktan sonra diskte değiştiyse üzerine yazmaz (çakışma).
import { Eye, Pencil } from 'lucide-react'
import { useCallback, useState } from 'react'
import useSWR from 'swr'
import type { MemoryDocFile, MemoryOverview } from '../../../ipc-memory.ts'
import { errorMessage } from '../../api.ts'
import { CodeEditor } from '../../CodeEditor.tsx'
import { Markdown } from '../../Markdown.tsx'
import { projmemApi } from './api.ts'

const DOC_LABEL: Record<MemoryDocFile, string> = {
  'architecture.md': 'Mimari',
  'decisions.md': 'Kararlar',
  'patterns.md': 'Kalıplar',
  'hotspots.md': 'Sıcak noktalar',
}

type Selection = { kind: 'doc'; file: MemoryDocFile } | { kind: 'session'; day: string }

export function DocsTab({ overview, onChanged }: { overview: MemoryOverview; onChanged: () => void }) {
  const [sel, setSel] = useState<Selection>({ kind: 'doc', file: 'architecture.md' })
  return (
    <div className="g14-section">
      <div className="kmem-chips" role="group" aria-label="Hafıza belgeleri">
        {overview.docs.map((d) => (
          <button
            key={d.file}
            type="button"
            className={`btn small${sel.kind === 'doc' && sel.file === d.file ? ' primary' : ''}`}
            aria-pressed={sel.kind === 'doc' && sel.file === d.file}
            onClick={() => setSel({ kind: 'doc', file: d.file })}
            title={d.file}
          >
            {DOC_LABEL[d.file]}
          </button>
        ))}
        <label className="kmem-inline">
          <span className="muted">Oturumlar</span>
          <select
            className="text-input"
            value={sel.kind === 'session' ? sel.day : ''}
            onChange={(e) => e.target.value && setSel({ kind: 'session', day: e.target.value })}
            disabled={overview.sessions.length === 0}
            data-kor-cmd="projmem.session"
          >
            <option value="">{overview.sessions.length === 0 ? 'Henüz yok' : 'Gün seç'}</option>
            {overview.sessions.map((d) => (
              <option key={d} value={d}>
                {d === 'undated' ? 'Tarihsiz' : d}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className="g7-note">
        Klasör: <code translate="no">{overview.dir}</code>
      </p>
      {sel.kind === 'doc' ? (
        <DocView key={`${overview.project}:${sel.file}`} project={overview.project} file={sel.file} onChanged={onChanged} />
      ) : (
        <SessionView key={sel.day} day={sel.day} />
      )}
    </div>
  )
}

function DocView({ project, file, onChanged }: { project: string; file: MemoryDocFile; onChanged: () => void }) {
  const [editing, setEditing] = useState(false)
  const { data, error, mutate } = useSWR(['projmem:read', project, file], () => projmemApi.read(file))
  const refresh = useCallback(() => {
    void mutate()
    onChanged()
  }, [mutate, onChanged])
  const write = useCallback((content: string, expected: number | null) => projmemApi.write(file, content, expected), [file])

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>
  return (
    <div className="g14-section" data-kor-cmd="projmem.read">
      <div className="kmem-row">
        <h3 className="g7-h4" translate="no">{file}</h3>
        <button
          type="button"
          className="btn small"
          onClick={() => setEditing((v) => !v)}
          aria-pressed={editing}
          data-kor-cmd="projmem.write"
        >
          {editing ? <Eye size={14} aria-hidden="true" /> : <Pencil size={14} aria-hidden="true" />} {editing ? 'Önizle' : 'Düzenle'}
        </button>
      </div>
      {data.sections.length > 0 ? (
        <ul className="core-list" aria-label="Kor'un bölümleri">
          {data.sections.map((s) => (
            <li key={`${s.name}:${s.start}`} className="core-row">
              <span className="core-row-main">
                <code translate="no">{s.name}</code> <span className="muted">{`satır ${s.start}-${s.end}`}</span>
              </span>
              <span className={`badge${s.edited ? ' kmem-edited' : ' neutral'}`}>
                {s.edited ? 'Düzenlendi: Kor dokunmaz' : 'Kor yönetiyor'}
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="g7-note">Bu belgede Kor'un işaretli bölümü yok; tamamı senin.</p>
      )}
      <p className="g7-note">
        Kor yalnızca kendi işaretli bölümlerini yeniden yazar; bir bölümün içini değiştirirsen o bölümü artık senin kabul eder.
        Belge sen düzenlerken diskte değişirse kayıt üzerine yazmaz, uyarır. Şifre ve anahtarlar kayıtta maskelenir.
      </p>
      {editing ? (
        <div className="kmem-editor">
          <CodeEditor
            path={`kor-memory/${project}/${file}`}
            content={data.content}
            mtimeMs={data.mtimeMs}
            line={null}
            lineSeq={0}
            onRefresh={refresh}
            onClose={() => setEditing(false)}
            write={write}
          />
        </div>
      ) : (
        <div className="kmem-preview" translate="no">
          <Markdown text={data.content} />
        </div>
      )}
    </div>
  )
}

function SessionView({ day }: { day: string }) {
  const { data, error } = useSWR(['projmem:session', day], () => projmemApi.session(day))
  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>
  return (
    <div className="g14-section" data-kor-cmd="projmem.session">
      <p className="g7-note">Oturum özetleri Kor tarafından eklenir ve burada salt okunurdur.</p>
      <div className="kmem-preview" translate="no">
        <Markdown text={data.content} />
      </div>
    </div>
  )
}
