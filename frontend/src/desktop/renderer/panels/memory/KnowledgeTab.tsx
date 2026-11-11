// Bilgi tabanı (§31.3): arama boşken zaman çizelgesi (son kayıtlar), yazınca `kor recall`; proje süzgeci,
// `kor forget` (onaylı) ve `kor knowledge export` (Markdown/JSON, seçilen klasöre).
import { FileDown, Search, Trash2 } from 'lucide-react'
import { useDeferredValue, useId, useState } from 'react'
import useSWR from 'swr'
import type { KnowledgeRecord } from '../../../ipc-memory.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { memoryApi, projmemApi } from './api.ts'
import { baseName, recordDate, type ProjectChoice } from './shape.ts'

export function KnowledgeTab({ choices }: { choices: ProjectChoice[] }) {
  const id = useId()
  const [query, setQuery] = useState('')
  const [project, setProject] = useState('')
  const q = useDeferredValue(query.trim())
  const only = project || null
  const { data, error, mutate } = useSWR(['memory:kb', q, only], async (): Promise<Array<{ record: KnowledgeRecord; score: number | null }>> =>
    q
      ? (await memoryApi.recall(q, only, 50)).hits
      : (await projmemApi.recent(only, 50)).records.map((record) => ({ record, score: null })),
  )
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const [busy, setBusy] = useState(false)

  const forget = async (r: KnowledgeRecord) => {
    const ok = await confirmAction({
      title: 'Kayıt silinsin mi?',
      message: `"${r.task || r.summary.slice(0, 80)}" bilgi tabanından kalıcı olarak silinecek. Geri alınamaz.`,
      confirmLabel: 'Sil',
      danger: true,
    })
    if (!ok) return
    memoryApi
      .forget(r.id)
      .then((res) => {
        setNote(res.deleted ? { text: 'Kayıt silindi.', ok: true } : { text: 'Kayıt zaten yoktu.', ok: false })
        void mutate()
      })
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
  }

  const exportAs = async (format: 'md' | 'json') => {
    setNote(null)
    setBusy(true)
    try {
      const dir = await projmemApi.pickFolder()
      if (!dir) return
      const r = await memoryApi.export(format, dir)
      setNote({ text: `${r.count} kayıt dışa aktarıldı: ${r.dir}`, ok: true })
    } catch (e) {
      setNote({ text: errorMessage(e), ok: false })
    } finally {
      setBusy(false)
    }
  }

  const sem = useSWR(['memory:semantic'], () => memoryApi.semantic(), { revalidateOnFocus: false })
  const reindex = async () => {
    const s = sem.data
    if (!s) return
    const ok = await confirmAction({
      title: 'Bilgi tabanı yeniden gömülsün mü?',
      message: `${s.records - s.embedded} kayıt yerel modelle gömülecek. Bu bilgisayarda çalışır, hiçbir şey dışarı gönderilmez; çok kayıtta uzun sürebilir.`,
      confirmLabel: 'Göm',
    })
    if (!ok) return
    setNote(null)
    setBusy(true)
    try {
      const r = await memoryApi.reindex()
      setNote({ text: `${r.embedded} kayıt gömüldü (toplam ${r.total}).`, ok: true })
      void sem.mutate()
    } catch (e) {
      setNote({ text: errorMessage(e), ok: false })
    } finally {
      setBusy(false)
    }
  }

  const rows = data ?? []
  return (
    <div className="g14-section">
      <div className="kmem-row kmem-wrap">
        <label className="kmem-search" htmlFor={`${id}-q`}>
          <Search size={14} aria-hidden="true" />
          <span className="sr-only">Bilgi tabanında ara</span>
          <input
            id={`${id}-q`}
            className="text-input"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Daha önce iyzico entegrasyonu yaptık mı?"
            maxLength={500}
            data-kor-cmd="memory.recall"
          />
        </label>
        <label className="kmem-inline">
          <span className="sr-only">Proje süzgeci</span>
          <select className="text-input" value={project} onChange={(e) => setProject(e.target.value)} aria-label="Proje süzgeci">
            <option value="">Tüm projeler</option>
            {choices.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className="g7-note" data-kor-cmd="projmem.recent">{q ? 'Arama sonuçları (açık proje öne çıkar).' : 'Zaman çizelgesi: en yeni kayıtlar.'}</p>
      {sem.data && (
        <div className="kmem-row kmem-wrap" data-kor-cmd="memory.semantic">
          <p className="g7-note">
            {sem.data.installed
              ? `Anlamsal arama açık: ${sem.data.embedded}/${sem.data.records} kayıt gömülü.`
              : `Anlamsal arama kapalı: yerel model (${sem.data.component}) kurulu değil, yalnızca sözcük araması.`}
          </p>
          {sem.data.installed && sem.data.embedded < sem.data.records && (
            <button type="button" className="btn small" disabled={busy} onClick={() => void reindex()} data-kor-cmd="memory.reindex">
              Eksikleri göm
            </button>
          )}
        </div>
      )}

      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted">Yükleniyor...</p>
      ) : rows.length === 0 ? (
        <p className="g7-note">{q ? 'Eşleşen kayıt yok.' : 'Bilgi tabanı boş. Kor her görevin sonunda kısa bir "ne öğrendik" kaydı ekler.'}</p>
      ) : (
        <ol className="core-list" data-kor-cmd={q ? 'memory.recall' : 'projmem.recent'}>
          {rows.map(({ record: r }) => (
            <li key={r.id} className="kmem-card">
              <div className="kmem-row">
                <span className="core-row-main">
                  <strong translate="no">{r.task || r.kind}</strong>
                </span>
                <button
                  type="button"
                  className="action-btn"
                  aria-label="Kaydı sil"
                  title="Kaydı sil (kor forget)"
                  onClick={() => void forget(r)}
                  data-kor-cmd="memory.forget"
                >
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </div>
              <p className="g7-note">
                <span className="g7-mono">{recordDate(r.ts)}</span> · <span translate="no">{baseName(r.project)}</span> · <span className="g7-mono">#{r.id}</span>
              </p>
              {r.summary && <p className="kmem-body" translate="no">{r.summary}</p>}
              {r.files.length > 0 && (
                <p className="g7-note">
                  Dosyalar: <span className="g7-mono" translate="no">{r.files.slice(0, 8).join(', ')}{r.files.length > 8 ? ' …' : ''}</span>
                </p>
              )}
              {r.errors.length > 0 && (
                <p className="g7-note">
                  Çözülen hatalar: <span translate="no">{r.errors.slice(0, 3).join(' · ')}</span>
                </p>
              )}
            </li>
          ))}
        </ol>
      )}

      <section className="g14-section g7-subsection" aria-labelledby={`${id}-export`} data-kor-cmd="memory.export">
        <h3 className="g7-h4" id={`${id}-export`}>Dışa aktar</h3>
        <p className="g7-note">Tüm kayıtlar seçtiğin klasöre: kayıt başına bir Markdown dosyası (Obsidian uyumlu) ya da tek knowledge.json.</p>
        <div className="kmem-row">
          <button type="button" className="btn small" disabled={busy} onClick={() => void exportAs('md')}>
            <FileDown size={14} aria-hidden="true" /> Markdown
          </button>
          <button type="button" className="btn small" disabled={busy} onClick={() => void exportAs('json')}>
            <FileDown size={14} aria-hidden="true" /> JSON
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
