// §27.15 kişisel bilgi tabanı (ADR 0063): doğrudan arama (eskimiş notlar işaretli, kaynak dosyalarıyla),
// haftalık "en çok başvurulan notlar" özeti, eskimiş notlar. CLI: kor know "<arama>" | digest | stale.
import { CalendarDays, Search, TriangleAlert } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { KnowHit, KnowStale } from '../../../ipc-ek14.ts'
import { errorMessage } from '../../api.ts'
import { knowApi } from '../../ek14-api.ts'
import type { KorPanelProps } from '../registry.ts'
import { reasonText } from './reasons.ts'

function StaleNote({ s }: { s: KnowStale }) {
  return (
    <p className="ks-warn">
      <TriangleAlert size={12} aria-hidden="true" /> Eskimiş olabilir: <span translate="no">{s.reasons.map(reasonText).join('; ')}</span>
    </p>
  )
}

function Hit({ h }: { h: KnowHit }) {
  const r = h.hit.record
  return (
    <li className="ks-entry">
      <div className="ks-entry-head">
        <strong translate="no">#{r.id} {r.task}</strong>
        <span className="ks-badge">{new Date(r.ts * 1000).toLocaleDateString()}</span>
        <span className="ks-chip" translate="no">{r.project.split('/').pop()}</span>
      </div>
      {r.summary && <p className="ks-entry-text" translate="no">{r.summary.split('\n').slice(0, 4).join('\n')}</p>}
      {r.files.length > 0 && <p className="ks-entry-meta muted" translate="no">{r.files.slice(0, 4).join(', ')}</p>}
      {h.stale && <StaleNote s={h.stale} />}
    </li>
  )
}

export function KnowledgePanel(_: KorPanelProps) {
  const uid = useId()
  const [q, setQ] = useState('')
  const [here, setHere] = useState(false)
  const [hits, setHits] = useState<KnowHit[] | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [digest, setDigest] = useState<{ week: string; markdown: string; path: string } | null>(null)
  const stale = useSWR(['know:stale', here], () => knowApi.stale(here), { revalidateOnFocus: false })
  const digests = useSWR('know:digests', () => knowApi.digests(), { revalidateOnFocus: false })
  const search = (e: FormEvent) => {
    e.preventDefault()
    if (!q.trim()) return
    setErr(null)
    knowApi.search(q.trim(), here).then(setHits).catch((x: unknown) => setErr(errorMessage(x)))
  }
  const openDigest = (week?: string) =>
    knowApi
      .digest(week ?? null)
      .then((d) => {
        setDigest(d)
        void digests.mutate()
      })
      .catch((x: unknown) => setErr(errorMessage(x)))
  return (
    <section className="ks" aria-label="Bilgi tabanı">
      <h2 className="ks-title">Bilgi tabanı</h2>
      <form className="ks-inline" onSubmit={search}>
        <label className="sr-only" htmlFor={`${uid}-q`}>Ara</label>
        <input id={`${uid}-q`} className="text-input" value={q} onChange={(e) => setQ(e.target.value)} placeholder="docker build cache" />
        <button type="submit" className="action-btn" aria-label="Notlarda ara" title="Ara" data-kor-cmd="know.search"><Search size={16} aria-hidden="true" /></button>
      </form>
      <label className="ks-check"><input type="checkbox" checked={here} onChange={(e) => setHere(e.target.checked)} /> Yalnızca bu proje</label>
      {err && <p className="error-text" role="alert">{err}</p>}
      {hits && (hits.length === 0 ? <p className="ks-empty">Eşleşen not yok.</p> : <ul className="ks-list" data-kor-cmd="know.search">{hits.map((h) => <Hit key={h.hit.record.id} h={h} />)}</ul>)}

      <div className="kg-card-head">
        <h3 className="ks-title">Haftalık özet</h3>
        <button type="button" className="btn" onClick={() => void openDigest()} data-kor-cmd="know.digest"><CalendarDays size={14} aria-hidden="true" /> Geçen hafta</button>
      </div>
      {digests.data && digests.data.length > 0 && (
        <div className="ks-actions" data-kor-cmd="know.digests">
          {digests.data.slice(0, 8).map((d) => (
            <button key={d.week} type="button" className="btn ghost" onClick={() => void openDigest(d.week)} translate="no">{d.week}</button>
          ))}
        </div>
      )}
      {digest && (
        <>
          <p className="ks-label" translate="no">{digest.path}</p>
          <pre className="ks-code" translate="no">{digest.markdown}</pre>
        </>
      )}

      <h3 className="ks-title">Eskimiş notlar</h3>
      {stale.error ? (
        <p className="error-text" role="alert">{errorMessage(stale.error)}</p>
      ) : !stale.data ? (
        <p className="muted" role="status" data-kor-cmd="know.stale">Notlar denetleniyor…</p>
      ) : stale.data.length === 0 ? (
        <p className="ks-empty">Dosyası ya da sembolü değişmiş not yok.</p>
      ) : (
        <ul className="ks-list" data-kor-cmd="know.stale">
          {stale.data.map((s) => (
            <li key={s.id} className="ks-entry">
              <strong translate="no">#{s.id} {s.task}</strong>
              <StaleNote s={s} />
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
