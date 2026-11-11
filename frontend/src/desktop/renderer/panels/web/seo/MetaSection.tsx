// Toplu meta ve alt metin (§29.8): önce örnek (20 satır) üretilir, tabloda düzeltilir; düzeltilen satırlar
// stil örneği olarak kalanların üretimine girer. Uzunluk/piksel/yinelenme kontrolü, CSV, WordPress'e onaylı
// yazım (her değişiklikten önce geri alma dosyası) ve geri alma. Ucuz model rotası (routing.simple_edit).
import { Download, FileUp, RotateCcw, Sparkles, Upload, Wand2 } from 'lucide-react'
import { useId, useState } from 'react'
import type { SeoMetaEstimate, SeoMetaRow } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { confirmAction } from '../../../ConfirmDialog.tsx'
import { seoApi } from './api.ts'
import { usd } from './format.ts'

const SHOWN = 200

export function MetaSection() {
  const uid = useId()
  const [url, setUrl] = useState('')
  const [rows, setRows] = useState<SeoMetaRow[]>([])
  const [csv, setCsv] = useState('.kor/seo/meta/meta.csv')
  const [wpSite, setWpSite] = useState('')
  const [keys, setKeys] = useState<'yoast' | 'rankmath' | 'none'>('yoast')
  const [undo, setUndo] = useState('')
  const [estimate, setEstimate] = useState<SeoMetaEstimate | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)

  const act = (f: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    setNote(null)
    f().catch((e: unknown) => setError(errorMessage(e))).finally(() => setBusy(false))
  }
  const edit = (i: number, field: 'title' | 'description' | 'alt', value: string) =>
    setRows((r) => r.map((x, j) => (j === i ? { ...x, [field]: value } : x)))
  const open = rows.filter((r) => (r.kind === 'image' ? !r.alt : !r.title))

  const collect = () => act(async () => setRows(await seoApi.metaCollect(url.trim(), false)))
  const generate = (all: boolean) => act(async () => {
    const batch = all ? open : open.slice(0, 20)
    const e = await seoApi.metaEstimate(batch, null)
    setEstimate(e)
    const ok = await confirmAction({
      title: 'Meta üret',
      message: `${batch.length} satır ${e.model} ile üretilecek (${usd(e.usd)}, tahmini).`,
      confirmLabel: 'Üret',
    })
    if (!ok) return
    setRows(await seoApi.metaGenerate(null, rows, 20, all, false, null, null))
  })
  const apply = () => act(async () => {
    const n = rows.filter((r) => r.title || r.alt).length
    const ok = await confirmAction({
      title: 'WordPress\'e yaz',
      message: `${n} satır ${wpSite} sitesine yazılacak (${keys}). Eski değerler önce geri alma dosyasına kaydedilir.`,
      confirmLabel: 'Yaz',
      danger: true,
    })
    if (!ok) return
    const r = await seoApi.metaApply(rows, wpSite.trim(), keys)
    setUndo(r.undo)
    setNote(`${r.applied.changed.length} güncellendi, ${r.applied.skipped.length} atlandı${r.applied.skipped.length ? `: ${r.applied.skipped.slice(0, 5).map(([u, w]) => `${u} (${w})`).join('; ')}` : ''}. Geri alma: ${r.undo}`)
  })
  const revert = () => act(async () => {
    const ok = await confirmAction({ title: 'Geri al', message: `${undo} içindeki eski değerler ${wpSite} sitesine geri yazılacak.`, confirmLabel: 'Geri al', danger: true })
    if (!ok) return
    const n = await seoApi.metaUndo(undo, wpSite.trim())
    setNote(`${n} değer geri yüklendi.`)
  })

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`} data-kor-event="kor:seo/progress">
      <h3 id={`${uid}-h`} className="kg-card-title ks-title">
        <Wand2 size={16} aria-hidden="true" />
        Toplu meta ve alt metin
      </h3>
      <div className="kp-form">
        <label className="kg-field kp-grow">
          <span>Site (taranır)</span>
          <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="ornek.com" spellCheck={false} maxLength={300} translate="no" disabled={rows.length > 0} />
        </label>
        <button type="button" className="btn small ghost" disabled={busy || rows.length > 0 || !url.trim()} onClick={collect} data-kor-cmd="seo.metaCollect">Sayfaları topla</button>
        <button type="button" className="btn small" disabled={busy || open.length === 0} onClick={() => generate(false)} data-kor-cmd="seo.metaGenerate">
          <Sparkles size={14} aria-hidden="true" />
          20 satır üret
        </button>
        <button type="button" className="btn small" disabled={busy || rows.length === 0 || open.length === 0} onClick={() => generate(true)} data-kor-cmd="seo.metaEstimate">Kalanların hepsi</button>
        <button type="button" className="btn small ghost" disabled={busy || rows.length === 0} onClick={() => act(async () => setRows(await seoApi.metaValidate(rows)))} data-kor-cmd="seo.metaValidate">Kontrol et</button>
      </div>
      <div className="kp-form">
        <label className="kg-field kp-grow">
          <span>CSV</span>
          <input value={csv} onChange={(e) => setCsv(e.target.value)} spellCheck={false} translate="no" />
        </label>
        <button type="button" className="btn small ghost" disabled={busy || rows.length === 0} onClick={() => act(async () => setNote(`Yazıldı: ${await seoApi.metaExport(rows, csv)}`))} data-kor-cmd="seo.metaExport">
          <Download size={14} aria-hidden="true" />
          CSV'ye yaz
        </button>
        <button type="button" className="btn small ghost" disabled={busy} onClick={() => act(async () => setRows(await seoApi.metaImport(csv)))} data-kor-cmd="seo.metaImport">
          <FileUp size={14} aria-hidden="true" />
          CSV'den oku
        </button>
      </div>
      {estimate && <p className="muted kp-small" translate="no">{`${estimate.model}: ≈ ${estimate.inputTokens} girdi + ${estimate.outputTokens} çıktı belirteci, ${usd(estimate.usd)}`}</p>}
      {rows.length === 0 ? (
        <p className="ks-empty">Bir site girip sayfaları toplayın (alt metni olmayan görseller dahil); önce 20 satır önizleme olarak üretilir, düzelttiğiniz satırlar kalanlar için stil örneği olur.</p>
      ) : (
        <div className="kp-table-wrap">
          <table className="kp-table">
            <caption className="kp-caption">{`${rows.length} satır · ${open.length} açık`}</caption>
            <thead>
              <tr><th scope="col">URL</th><th scope="col">Başlık / alt metin</th><th scope="col">Açıklama</th><th scope="col">Sorunlar</th></tr>
            </thead>
            <tbody>
              {rows.slice(0, SHOWN).map((r, i) => (
                <tr key={`${r.kind}-${r.url}`}>
                  <td className="kp-loc" translate="no">{r.kind === 'image' ? `Görsel: ${r.url}` : r.url}</td>
                  <td>
                    <input className="kseo-cell" aria-label={r.kind === 'image' ? 'Alt metin' : 'Başlık'} value={r.kind === 'image' ? r.alt : r.title} onChange={(e) => edit(i, r.kind === 'image' ? 'alt' : 'title', e.target.value)} translate="no" />
                    <div className="muted kseo-fix" translate="no">{r.kind === 'image' ? r.current_alt : r.current_title}</div>
                  </td>
                  <td>
                    {r.kind === 'page' && <input className="kseo-cell" aria-label="Açıklama" value={r.description} onChange={(e) => edit(i, 'description', e.target.value)} translate="no" />}
                  </td>
                  <td className="muted kp-small" translate="no">{r.issues}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length > SHOWN && <p className="muted kp-small">{`İlk ${SHOWN} satır düzenlenebilir; tamamı CSV'de.`}</p>}
        </div>
      )}
      <div className="kp-form">
        <label className="kg-field">
          <span>WordPress sitesi</span>
          <input value={wpSite} onChange={(e) => setWpSite(e.target.value)} placeholder="site-adi" spellCheck={false} translate="no" />
        </label>
        <label className="kg-field">
          <span>SEO alanları</span>
          <select value={keys} onChange={(e) => setKeys(e.target.value as typeof keys)}>
            <option value="yoast">Yoast</option>
            <option value="rankmath">Rank Math</option>
            <option value="none">Yalnız özet (excerpt)</option>
          </select>
        </label>
        <button type="button" className="btn small" disabled={busy || !wpSite.trim() || rows.length === 0} onClick={apply} data-kor-cmd="seo.metaApply">
          <Upload size={14} aria-hidden="true" />
          WordPress'e yaz
        </button>
        <label className="kg-field kp-grow">
          <span>Geri alma dosyası</span>
          <input value={undo} onChange={(e) => setUndo(e.target.value)} spellCheck={false} translate="no" />
        </label>
        <button type="button" className="btn small ghost" disabled={busy || !undo.trim() || !wpSite.trim()} onClick={revert} data-kor-cmd="seo.metaUndo">
          <RotateCcw size={14} aria-hidden="true" />
          Geri al
        </button>
      </div>
      <p className="ks-note">Eklenti alanları REST API'ye açık değilse WordPress onları sessizce yok sayar; Kor her yazımı geri okur ve tutmayanları "atlandı" olarak bildirir.</p>
      {busy && <p className="muted kp-small" role="status">Çalışıyor…</p>}
      {note && <p className="muted kp-small" role="status" translate="no">{note}</p>}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
    </section>
  )
}
