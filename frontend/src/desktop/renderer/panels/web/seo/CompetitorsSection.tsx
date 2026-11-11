// Rakip izleme (§29.10): rakip sayfaların başlık, açıklama, başlıklar, kelime sayısı, schema ve fiyat anlık
// görüntüleri ile site haritası URL kümesi; her koşu öncekiyle karşılaştırılır. robots.txt'e uyar, hız sınırlı.
import { CalendarClock, Eye, Plus, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { SeoCompDiff } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { confirmAction } from '../../../ConfirmDialog.tsx'
import { seoApi } from './api.ts'
import { parseUrls } from './format.ts'

export function CompetitorsSection() {
  const uid = useId()
  const list = useSWR('seo.competitorsList', () => seoApi.competitorsList())
  const [name, setName] = useState('')
  const [urls, setUrls] = useState('')
  const [ours, setOurs] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [diffs, setDiffs] = useState<SeoCompDiff[] | null>(null)

  const act = (f: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    setNote(null)
    f().catch((e: unknown) => setError(errorMessage(e))).finally(() => { setBusy(false); void list.mutate() })
  }
  const add = (e: FormEvent) => {
    e.preventDefault()
    act(async () => {
      await seoApi.competitorsAdd(name.trim(), parseUrls(urls), ours.trim() || null)
      setName('')
      setUrls('')
    })
  }
  const remove = (n: string) => act(async () => {
    if (await confirmAction({ title: 'Rakibi kaldır', message: `${n} izleme listesinden çıkarılacak (eski anlık görüntüler kalır).`, confirmLabel: 'Kaldır', danger: true })) {
      await seoApi.competitorsRemove(n)
    }
  })
  const comps = list.data?.competitor ?? []

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`} data-kor-event="kor:seo/progress">
      <h3 id={`${uid}-h`} className="kg-card-title ks-title">
        <Eye size={16} aria-hidden="true" />
        Rakip izleme
      </h3>
      <form className="ks-form" onSubmit={add}>
        <div className="kp-form">
          <label className="kg-field">
            <span>Ad</span>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="rakip-1" pattern="[A-Za-z0-9_-]{1,64}" required />
          </label>
          <label className="kg-field kp-grow">
            <span>Bizim sayfamız (eksik raporu, isteğe bağlı)</span>
            <input value={ours} onChange={(e) => setOurs(e.target.value)} placeholder="https://ornek.com/kargo" spellCheck={false} translate="no" />
          </label>
        </div>
        <label className="ks-field">
          <span>Rakip sayfalar (satır başına bir URL)</span>
          <textarea className="kseo-text kseo-short" value={urls} onChange={(e) => setUrls(e.target.value)} spellCheck={false} translate="no" required />
        </label>
        <button type="submit" className="btn small" disabled={busy || !name.trim() || parseUrls(urls).length === 0} data-kor-cmd="seo.competitorsAdd">
          <Plus size={14} aria-hidden="true" />
          Ekle
        </button>
      </form>
      {list.error ? (
        <p className="error-text" role="alert" translate="no">{errorMessage(list.error)}</p>
      ) : !list.data ? (
        <p className="muted" role="status" data-kor-cmd="seo.competitorsList">Liste okunuyor…</p>
      ) : comps.length === 0 ? (
        <p className="ks-empty" data-kor-cmd="seo.competitorsList">Henüz rakip yok. Ayarlar <code>.kor/seo/competitors.toml</code> dosyasında.</p>
      ) : (
        <ul className="ks-list" data-kor-cmd="seo.competitorsList">
          {comps.map((c) => (
            <li key={c.name} className="kg-card-head">
              <span translate="no">{`${c.name}: ${c.urls.length} sayfa${c.ours ? ` ↔ ${c.ours}` : ''}`}</span>
              <button type="button" className="action-btn" onClick={() => remove(c.name)} aria-label={`${c.name} rakibini kaldır`} title="Kaldır" data-kor-cmd="seo.competitorsRemove">
                <Trash2 size={16} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="kseo-tabs">
        <button type="button" className="btn small" disabled={busy || comps.length === 0} onClick={() => act(async () => setDiffs(await seoApi.competitorsRun(null)))} data-kor-cmd="seo.competitorsRun">
          <Eye size={14} aria-hidden="true" />
          {busy ? 'Taranıyor…' : 'Şimdi tara'}
        </button>
        {(['daily', 'weekly'] as const).map((every) => (
          <button key={every} type="button" className="btn small ghost" disabled={busy || comps.length === 0} onClick={() => act(async () => { await seoApi.competitorsSchedule(every); setNote(every === 'daily' ? 'Günlük izleme rutini eklendi.' : 'Haftalık izleme rutini eklendi.') })} data-kor-cmd="seo.competitorsSchedule">
            <CalendarClock size={14} aria-hidden="true" />
            {every === 'daily' ? 'Günlük' : 'Haftalık'}
          </button>
        ))}
      </div>
      {note && <p className="muted kp-small" role="status">{note}</p>}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
      {diffs?.map((d) => (
        <div key={d.name}>
          <h4 className="kp-h" translate="no">{d.name}</h4>
          {d.new_urls.length + d.removed_urls.length + d.changes.length + d.gaps.length === 0 ? (
            <p className="muted kp-small">Önceki anlık görüntüden beri değişiklik yok (ilk koşu yalnız temel alır).</p>
          ) : (
            <ul className="kp-notes" translate="no">
              {d.new_urls.slice(0, 30).map((u) => <li key={`+${u}`}>{`+ ${u}`}</li>)}
              {d.removed_urls.slice(0, 30).map((u) => <li key={`-${u}`}>{`− ${u}`}</li>)}
              {d.changes.map((c, i) => <li key={`c${i}`}>{`${c.url} ${c.field}: "${c.before}" → "${c.after}"`}</li>)}
              {d.gaps.map((g, i) => <li key={`g${i}`}>{`Onlarda var, bizde yok: ${g}`}</li>)}
            </ul>
          )}
        </div>
      ))}
    </section>
  )
}
