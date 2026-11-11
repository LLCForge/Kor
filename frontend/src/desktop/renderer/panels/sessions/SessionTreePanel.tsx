// Oturum ağacı (§28.6, ADR 0050): klasör, çoklu etiket, renk, sabitleme, arşiv, tam metin + anlamsal arama,
// başlık/etiket önerisi, Markdown dışa/içe aktarma. Kayıtlar kenar çubuğuyla ve `kor sessions` ile ortaktır.
import { Archive, FileDown, FileUp, Pin, RefreshCw, Search, Sparkles, Tag } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { SESSION_COLORS, type SessionColor, type SessionEntry, type SessionHit } from '../../../ipc-prefs.ts'
import { api, errorMessage } from '../../api.ts'
import { sessionsApi } from '../../prefs/api.ts'
import type { KorPanelProps } from '../registry.ts'
import { OPEN_TARGET_EVENT } from '../../shellui/bridge.ts'

/** Sohbeti uygulamada açar (başlık çubuğu kuyruğuyla aynı yol). */
const openSession = (sessionId: string) => window.dispatchEvent(new CustomEvent(OPEN_TARGET_EVENT, { detail: { sessionId } }))
import '../graph/panel.css'
import '../../prefs/prefs.css'

const COLOR_LABEL: Record<SessionColor, string> = { red: 'Kırmızı', orange: 'Turuncu', yellow: 'Sarı', green: 'Yeşil', blue: 'Mavi', purple: 'Mor', gray: 'Gri' }

function Row({ e, onChanged }: { e: SessionEntry; onChanged: () => void }) {
  const uid = useId()
  const [error, setError] = useState<string | null>(null)
  const [tagText, setTagText] = useState('')
  const [folder, setFolder] = useState(e.folder ?? '')
  const act = async (p: Promise<unknown>) => {
    setError(null)
    try {
      await p
      onChanged()
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const addTags = (ev: FormEvent) => {
    ev.preventDefault()
    const tags = tagText.split(/[\s,]+/).filter(Boolean)
    if (tags.length === 0) return
    setTagText('')
    void act(sessionsApi.tag(e.id, tags, []))
  }
  return (
    <li className="kg-card" aria-labelledby={`${uid}-t`}>
      <div className="kg-card-head">
        <span className={`kor-dot ${e.color ?? ''}`} aria-hidden="true" />
        <button type="button" id={`${uid}-t`} className="kg-link kg-card-name" translate="no" onClick={() => openSession(e.id)} title="Sohbeti aç">
          {e.title || e.id}
        </button>
        <button type="button" className={`action-btn${e.pinned ? ' active' : ''}`} aria-pressed={e.pinned} aria-label={e.pinned ? 'Sabitlemeyi kaldır' : 'Sabitle'} title={e.pinned ? 'Sabitlemeyi kaldır' : 'Sabitle'} data-kor-cmd="sessions.pin" onClick={() => void act(sessionsApi.pin(e.id, !e.pinned))}>
          <Pin size={14} aria-hidden="true" />
        </button>
        <button type="button" className="action-btn" aria-label={e.archived ? 'Arşivden çıkar' : 'Arşivle'} title={e.archived ? 'Arşivden çıkar' : 'Arşivle'} data-kor-cmd="sessions.archive" onClick={() => void act(sessionsApi.archive(e.id, !e.archived))}>
          <Archive size={14} aria-hidden="true" />
        </button>
        <button type="button" className="action-btn" aria-label="Başlık ve etiket öner ve uygula" title="Başlık ve etiket öner" data-kor-cmd="sessions.suggest" onClick={() => void act(sessionsApi.suggest(e.id, true))}>
          <Sparkles size={14} aria-hidden="true" />
        </button>
      </div>
      <div className="kg-row">
        {e.tags.map((t) => (
          <button key={t} type="button" className="kor-tag" title="Etiketi kaldır" aria-label={`#${t} etiketini kaldır`} data-kor-cmd="sessions.tag" onClick={() => void act(sessionsApi.tag(e.id, [], [t]))}>
            #{t} ×
          </button>
        ))}
        <form className="kg-row" onSubmit={addTags}>
          <input className="text-input" value={tagText} onChange={(ev) => setTagText(ev.target.value)} placeholder="#etiket" aria-label="Etiket ekle" maxLength={64} />
          <button type="submit" className="action-btn" aria-label="Etiket ekle" title="Etiket ekle" data-kor-cmd="sessions.tag"><Tag size={14} aria-hidden="true" /></button>
        </form>
      </div>
      <div className="kg-row">
        <label className="kg-field">
          Klasör
          <input className="text-input" value={folder} maxLength={60} onChange={(ev) => setFolder(ev.target.value)} onBlur={() => folder !== (e.folder ?? '') && void act(sessionsApi.mv(e.id, folder.trim() || null))} data-kor-cmd="sessions.mv" />
        </label>
        <label className="kg-field">
          Renk
          <select className="g7-select" value={e.color ?? ''} data-kor-cmd="sessions.color" onChange={(ev) => void act(sessionsApi.color(e.id, (ev.target.value || null) as SessionColor | null))}>
            <option value="">Yok</option>
            {SESSION_COLORS.map((c) => <option key={c} value={c}>{COLOR_LABEL[c]}</option>)}
          </select>
        </label>
        <label className="kg-field">
          Başlık
          <input className="text-input" defaultValue={e.title} maxLength={200} data-kor-cmd="sessions.rename" onBlur={(ev) => ev.target.value !== e.title && void act(sessionsApi.rename(e.id, ev.target.value.trim() || null))} />
        </label>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
    </li>
  )
}

export function SessionTreePanel(_props: KorPanelProps) {
  const [tag, setTag] = useState('')
  const [archived, setArchived] = useState(false)
  const list = useSWR(['sessions:tree', tag, archived], () => sessionsApi.list({ tag: tag || undefined, archived }))
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<SessionHit[] | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [importPath, setImportPath] = useState('')
  useEffect(() => api.onSessionsChanged(() => void list.mutate()), [list])

  const search = async (ev: FormEvent) => {
    ev.preventDefault()
    setNote(null)
    try {
      setHits(query.trim() ? await sessionsApi.search(query.trim()) : null)
    } catch (err) {
      setNote(errorMessage(err))
    }
  }
  const exportAll = async () => {
    try {
      const r = await sessionsApi.export()
      setNote(`${r.count} oturum Markdown olarak yazıldı: ${r.dir}`)
    } catch (err) {
      setNote(errorMessage(err))
    }
  }
  const importOne = async (ev: FormEvent) => {
    ev.preventDefault()
    try {
      const r = await sessionsApi.import(importPath.trim())
      setImportPath('')
      setNote(`İçe aktarıldı: ${r.id}`)
      void list.mutate()
    } catch (err) {
      setNote(errorMessage(err))
    }
  }
  const groups = new Map<string, SessionEntry[]>()
  for (const e of list.data ?? []) {
    const k = e.folder ?? 'Klasörsüz'
    groups.set(k, [...(groups.get(k) ?? []), e])
  }
  return (
    <div className="kp" data-kor-event="kor:sessionsChanged">
      <form className="kg-row" onSubmit={(ev) => void search(ev)}>
        <input className="text-input" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Sohbetlerde ara (ör. redis timeout)" aria-label="Sohbetlerde ara" maxLength={300} />
        <button type="submit" className="action-btn" aria-label="Ara" title="Ara" data-kor-cmd="sessions.search"><Search size={16} aria-hidden="true" /></button>
      </form>
      {hits && (
        <ul className="kg-list" aria-label="Arama sonuçları">
          {hits.length === 0 && <li className="muted">Eşleşme yok.</li>}
          {hits.map((h) => (
            <li key={h.id}>
              <button type="button" className="kg-link" onClick={() => openSession(h.id)} translate="no">{h.title || h.id}</button>
              {h.semantic && <span className="kor-tag" title="Anlamca yakın (yerel gömme modeli)">≈</span>}
              <p className="muted kg-count" translate="no">{h.snippet}</p>
            </li>
          ))}
        </ul>
      )}
      <div className="kg-row">
        <label className="kg-field">
          Etiket süzgeci
          <input className="text-input" value={tag} onChange={(e) => setTag(e.target.value)} placeholder="auth" maxLength={32} data-kor-cmd="sessions.list" />
        </label>
        <label className="kg-check">
          <input type="checkbox" checked={archived} onChange={(e) => setArchived(e.target.checked)} /> Arşivdekiler de
        </label>
        <button type="button" className="action-btn kg-push" onClick={() => void list.mutate()} aria-label="Yenile" title="Yenile" data-kor-cmd="sessions.list"><RefreshCw size={16} aria-hidden="true" /></button>
      </div>
      {list.error && <p className="error-text" role="alert">{errorMessage(list.error)}</p>}
      {!list.data && !list.error && <p className="muted">Yükleniyor…</p>}
      {list.data?.length === 0 && <div className="kg-empty"><p>Bu süzgece uyan oturum yok.</p></div>}
      {[...groups.entries()].map(([name, items]) => (
        <section key={name} aria-label={name}>
          <h3 className="kg-count">{name} · {items.length}</h3>
          <ul className="kg-list">
            {items.map((e) => <Row key={e.id} e={e} onChanged={() => void list.mutate()} />)}
          </ul>
        </section>
      ))}
      <div className="kg-row">
        <button type="button" className="btn" onClick={() => void exportAll()} data-kor-cmd="sessions.export"><FileDown size={14} aria-hidden="true" /> Markdown olarak dışa aktar</button>
        <form className="kg-row" onSubmit={(ev) => void importOne(ev)}>
          <input className="text-input" value={importPath} onChange={(e) => setImportPath(e.target.value)} placeholder="İçe aktarılacak .md dosyasının yolu" aria-label="İçe aktarılacak dosya" />
          <button type="submit" className="btn" disabled={!importPath.trim()} data-kor-cmd="sessions.import"><FileUp size={14} aria-hidden="true" /> İçe aktar</button>
        </form>
      </div>
      {note && <p className="g7-note" role="status">{note}</p>}
    </div>
  )
}
