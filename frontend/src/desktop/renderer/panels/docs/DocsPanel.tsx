// §28.7 özel dokümanlar paneli (ADR 0063): PDF/DOCX/Markdown/HTML dosyaları, Confluence/Notion dışa
// aktarım klasörleri, intranet URL'leri (gerekirse oturum açık Chrome ile) bilgi tabanına indekslenir;
// ajan bunları kendiliğinden hatırlar. CLI: kor docs add|list|search|refresh|rm.
import { FilePlus2, Globe, RefreshCw, Search, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { docsApi } from '../../ek14-api.ts'
import type { KorPanelProps } from '../registry.ts'

export function DocsPanel(_: KorPanelProps) {
  const uid = useId()
  const { data, error, mutate } = useSWR('docs:list', () => docsApi.list(), { revalidateOnFocus: false })
  const [source, setSource] = useState('')
  const [browser, setBrowser] = useState(false)
  const [q, setQ] = useState('')
  const [hits, setHits] = useState<{ title: string; text: string; source: string | null }[] | null>(null)
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const done = (p: Promise<unknown>, ok: string) => {
    setBusy(true)
    setNote(null)
    p.then((r) => {
      if (r !== null) setNote({ text: ok, ok: true })
      void mutate()
    })
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
      .finally(() => setBusy(false))
  }
  const add = (e: FormEvent) => {
    e.preventDefault()
    if (source.trim()) done(docsApi.add(source.trim(), browser), 'İndekslendi.')
  }
  const search = (e: FormEvent) => {
    e.preventDefault()
    if (q.trim()) docsApi.search(q.trim()).then(setHits).catch((x: unknown) => setNote({ text: errorMessage(x), ok: false }))
  }
  return (
    <section className="ks" aria-label="Dokümanlar">
      <div className="kg-card-head">
        <h2 className="ks-title">Dokümanlar</h2>
        <button type="button" className="action-btn" disabled={busy} onClick={() => done(docsApi.refresh(null), 'Değişenler yeniden indekslendi.')} aria-label="Değişen dokümanları yeniden indeksle" title="Yeniden indeksle" data-kor-cmd="docs.refresh">
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      <form className="ks-form" onSubmit={add}>
        <label className="ks-field" htmlFor={`${uid}-src`}>Dosya, klasör ya da URL</label>
        <div className="ks-inline">
          <input id={`${uid}-src`} className="text-input" value={source} onChange={(e) => setSource(e.target.value)} placeholder="./spec.pdf · ./wiki-export · https://intranet/wiki/Runbook" translate="no" />
          <button type="button" className="action-btn" disabled={busy} onClick={() => done(docsApi.pick(), 'İndekslendi.')} aria-label="Dosya seç ve indeksle" title="Dosya seç" data-kor-cmd="docs.pick">
            <FilePlus2 size={16} aria-hidden="true" />
          </button>
        </div>
        <label className="ks-check"><input type="checkbox" checked={browser} onChange={(e) => setBrowser(e.target.checked)} /> <Globe size={12} aria-hidden="true" /> Giriş gerektiren sayfayı oturum açık Chrome'umla oku</label>
        <button type="submit" className="btn primary" disabled={busy || !source.trim()} data-kor-cmd="docs.add">{busy ? 'İndeksleniyor…' : 'Ekle'}</button>
      </form>
      {note && <p className={note.ok ? 'ks-ok' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">Dokümanlar okunuyor…</p>
      ) : data.length === 0 ? (
        <div className="ks-empty">
          <p>Henüz doküman yok.</p>
          <p className="ks-note">Metin yerelde çıkarılır; gizlilik modunda yerel olmayan adreslere gidilmez, anlamsal arama yerel gömme modeliyle yapılır.</p>
        </div>
      ) : (
        <ul className="ks-list" data-kor-cmd="docs.list">
          {data.map((d) => (
            <li key={d.id} className="ks-row">
              <span className="kg-link-meta" translate="no" title={d.source}>{d.title}</span>
              <span className="ks-badge">{d.records.length} parça</span>
              <button type="button" className="action-btn reject" onClick={() => done(docsApi.remove(d.id), 'Kaldırıldı.')} aria-label={`${d.title} dokümanını kaldır`} title="Kaldır" data-kor-cmd="docs.remove">
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form className="ks-inline" onSubmit={search}>
        <label className="sr-only" htmlFor={`${uid}-q`}>Dokümanlarda ara</label>
        <input id={`${uid}-q`} className="text-input" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Dokümanlarda ara" />
        <button type="submit" className="action-btn" aria-label="Dokümanlarda ara" title="Ara" data-kor-cmd="docs.search"><Search size={16} aria-hidden="true" /></button>
      </form>
      {hits && (hits.length === 0 ? <p className="ks-empty">Eşleşme yok.</p> : (
        <ul className="ks-list">
          {hits.map((h, i) => (
            <li key={i} className="ks-entry">
              <strong translate="no">{h.title}</strong>
              <p className="ks-entry-text" translate="no">{h.text.slice(0, 600)}</p>
            </li>
          ))}
        </ul>
      ))}
    </section>
  )
}
