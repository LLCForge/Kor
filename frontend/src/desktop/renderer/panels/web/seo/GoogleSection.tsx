// Search Console + GA4 (§29.6) ve Sıralama Fırsat Skoru (§29.16). OAuth, kullanıcının kendi Google Cloud
// "Masaüstü uygulaması" istemcisiyle yapılır (Kor istemci kimliği içermez); belirteçler kasada, salt okunur.
import { BarChart3, BellRing, KeyRound, LogIn, LogOut, RefreshCw, Target, TrendingDown } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { SeoBingStat, SeoDrop, SeoOpportunity, SeoRow } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { seoApi } from './api.ts'
import { pct } from './format.ts'

const HINT: Record<string, string> = {
  position: 'sıralama düştü',
  demand: 'gösterim azaldı (talep ya da indeksleme)',
  snippet: 'aynı konumda TO düştü (başlık/snippet)',
  gone: 'artık gösterilmiyor',
}

type View =
  | { kind: 'rows'; rows: SeoRow[]; range: string }
  | { kind: 'drops'; drops: SeoDrop[]; range: string; hypotheses: string | null; alert: string | null }
  | { kind: 'bing'; rows: SeoBingStat[]; range: string }
  | { kind: 'ga'; rows: Record<string, string | number>[]; range: string }
  | { kind: 'opp'; rows: SeoOpportunity[]; formula: string; range: string; calibration: string | null }

function Setup({ onDone }: { onDone: () => void }) {
  const [json, setJson] = useState('')
  const [error, setError] = useState<string | null>(null)
  const save = () => {
    setError(null)
    seoApi.gscSetup(json).then(() => { setJson(''); onDone() }).catch((e: unknown) => setError(errorMessage(e)))
  }
  return (
    <div className="ks-form">
      <ol className="kp-notes">
        <li>console.cloud.google.com'da bir proje açın ya da seçin.</li>
        <li>"Google Search Console API" ve "Google Analytics Data API"yi etkinleştirin.</li>
        <li>OAuth izin ekranı: tür Harici; kendinizi test kullanıcısı olarak ekleyin.</li>
        <li>Kimlik bilgileri → OAuth istemci kimliği → tür "Masaüstü uygulaması"; JSON'u indirin.</li>
        <li>JSON içeriğini aşağıya yapıştırın. Kimlik ve gizli anahtar kasaya yazılır.</li>
      </ol>
      <label className="ks-field">
        <span>OAuth istemci JSON'u</span>
        <textarea className="kseo-text" value={json} onChange={(e) => setJson(e.target.value)} spellCheck={false} maxLength={20000} translate="no" placeholder='{"installed":{"client_id":"…apps.googleusercontent.com", …}}' />
      </label>
      <button type="button" className="btn small" disabled={!json.trim()} onClick={save} data-kor-cmd="seo.gscSetup">İstemciyi kaydet</button>
      {error && <p className="error-text" role="alert" translate="no">{error}</p>}
    </div>
  )
}

export function GoogleSection() {
  const uid = useId()
  const status = useSWR('seo.gscStatus', () => seoApi.gscStatus())
  const [site, setSite] = useState('')
  const [property, setProperty] = useState('')
  const [days, setDays] = useState('28')
  const [dims, setDims] = useState('query')
  const [sites, setSites] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [authUrl, setAuthUrl] = useState<string | null>(null)
  const [view, setView] = useState<View | null>(null)
  const [explain, setExplain] = useState(false)
  const [alertPct, setAlertPct] = useState('')
  const [why, setWhy] = useState(false)
  const [bingKey, setBingKey] = useState('')
  const [note, setNote] = useState<string | null>(null)
  const bing = useSWR('seo.bingStatus', () => seoApi.bingStatus())

  useEffect(() => seoApi.onOauth((e) => setAuthUrl(e.url)), [])

  const n = Math.max(1, Math.min(480, Number.parseInt(days, 10) || 28))
  const act = (f: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    f().catch((e: unknown) => setError(errorMessage(e))).finally(() => setBusy(false))
  }
  const login = () => act(async () => { await seoApi.gscLogin(); setAuthUrl(null); await status.mutate() })
  const logout = () => act(async () => { await seoApi.gscLogout(); await status.mutate() })

  const st = status.data
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`} data-kor-event="kor:seo/oauth">
      <div className="kg-card-head">
        <h3 id={`${uid}-h`} className="kg-card-title ks-title">
          <BarChart3 size={16} aria-hidden="true" />
          Search Console ve Analytics
        </h3>
        <button type="button" className="action-btn" onClick={() => void status.mutate()} aria-label="Bağlantı durumunu yenile" title="Yenile" data-kor-cmd="seo.gscStatus">
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      {status.error ? (
        <p className="error-text" role="alert" translate="no">{errorMessage(status.error)}</p>
      ) : !st ? (
        <p className="muted" role="status">Durum okunuyor…</p>
      ) : !st.client ? (
        <Setup onDone={() => void status.mutate()} />
      ) : !st.signed_in ? (
        <div className="ks-form">
          <p className="ks-note">İstemci hazır. Giriş tarayıcıda açılır; Kor yanıtı yalnız 127.0.0.1 üzerinde bekler (en çok 5 dakika).</p>
          <button type="button" className="btn small" disabled={busy} onClick={login} data-kor-cmd="seo.gscLogin">
            <LogIn size={14} aria-hidden="true" />
            {busy ? 'Tarayıcı bekleniyor…' : 'Google ile giriş yap'}
          </button>
          {authUrl && <p className="muted kp-small kp-loc" translate="no">{authUrl}</p>}
        </div>
      ) : (
        <>
          <div className="kp-form">
            <label className="kg-field kp-grow">
              <span>Search Console mülkü</span>
              <input value={site} onChange={(e) => setSite(e.target.value)} list={`${uid}-sites`} placeholder="sc-domain:ornek.com" spellCheck={false} maxLength={300} translate="no" />
              <datalist id={`${uid}-sites`}>{sites.map((x) => <option key={x} value={x} />)}</datalist>
            </label>
            <label className="kg-field kp-narrow">
              <span>Gün</span>
              <input value={days} onChange={(e) => setDays(e.target.value)} inputMode="numeric" />
            </label>
            <label className="kg-field">
              <span>Boyut</span>
              <select value={dims} onChange={(e) => setDims(e.target.value)}>
                <option value="query">Sorgu</option>
                <option value="page">Sayfa</option>
                <option value="query,page">Sorgu + sayfa</option>
                <option value="country">Ülke</option>
                <option value="device">Cihaz</option>
              </select>
            </label>
          </div>
          <div className="kseo-tabs">
            <button type="button" className="btn small ghost" disabled={busy} onClick={() => act(async () => setSites(await seoApi.gscSites()))} data-kor-cmd="seo.gscSites">Mülkleri getir</button>
            <button type="button" className="btn small" disabled={busy || !site.trim()} onClick={() => act(async () => { const r = await seoApi.gscQuery(site.trim(), n, dims, 200); setView({ kind: 'rows', rows: r.rows, range: `${r.start} … ${r.end}` }) })} data-kor-cmd="seo.gscQuery">Sorgula</button>
            <button type="button" className="btn small" disabled={busy || !site.trim()} onClick={() => act(async () => { const pctN = Number.parseFloat(alertPct); const r = await seoApi.gscDrops(site.trim(), Math.min(n, 90), 20, Number.isFinite(pctN) && pctN > 0 ? pctN : null, why); setView({ kind: 'drops', drops: r.drops, range: `${r.before.join('…')} → ${r.now.join('…')}`, hypotheses: r.hypotheses, alert: r.alert?.text ?? null }) })} data-kor-cmd="seo.gscDrops">
              <TrendingDown size={14} aria-hidden="true" />
              Tıklaması düşenler
            </button>
            <button type="button" className="btn small" disabled={busy || !site.trim()} onClick={() => act(async () => { const r = await seoApi.opportunities(site.trim(), n, dims, 50, false); setView({ kind: 'opp', rows: r.rows, formula: r.formula, range: `${r.start} … ${r.end}`, calibration: r.calibration ? `Geçmiş tahmin isabeti: ${r.calibration.checked} büyük fırsatın %${(r.calibration.hit_rate * 100).toFixed(0)} kadarı konumunu iyileştirdi (30+ gün sonra).` : null }) })} data-kor-cmd="seo.opportunities">
              <Target size={14} aria-hidden="true" />
              Fırsat skoru
            </button>
            <button type="button" className="btn small ghost" disabled={busy} onClick={logout} data-kor-cmd="seo.gscLogout">
              <LogOut size={14} aria-hidden="true" />
              Çıkış
            </button>
          </div>
          <div className="kp-form">
            <label className="kg-field kp-narrow">
              <span>Uyarı eşiği %</span>
              <input value={alertPct} onChange={(e) => setAlertPct(e.target.value)} inputMode="decimal" placeholder="20" />
            </label>
            <label className="kseo-check"><input type="checkbox" checked={why} onChange={(e) => setWhy(e.target.checked)} />Neden + aksiyon (model)</label>
            <button type="button" className="btn small ghost" disabled={busy || !site.trim()} onClick={() => act(async () => { await seoApi.gscDropsSchedule(site.trim(), Number.parseFloat(alertPct) || 20, 'daily'); setNote('Günlük düşüş izleme rutini eklendi.') })} data-kor-cmd="seo.gscDropsSchedule">
              <BellRing size={14} aria-hidden="true" />
              Günlük izle
            </button>
          </div>
          <div className="kp-form">
            <label className="kg-field">
              <span>GA4 mülk kimliği</span>
              <input value={property} onChange={(e) => setProperty(e.target.value.replace(/\D/g, ''))} inputMode="numeric" maxLength={20} placeholder="123456789" />
            </label>
            <button type="button" className="btn small" disabled={busy || !property} onClick={() => act(async () => { const r = await seoApi.gaReport(property, n, 'sessions,keyEvents', 'landingPage'); setView({ kind: 'ga', rows: r.rows, range: `${r.start} … ${r.end}` }) })} data-kor-cmd="seo.gaReport">GA4 raporu</button>
          </div>
        </>
      )}
      <div className="kp-form" data-kor-cmd="seo.bingStatus">
        {bing.data ? (
          <>
            <span className="kp-small">Bing Webmaster bağlı</span>
            <button type="button" className="btn small" disabled={busy || !site.trim()} onClick={() => act(async () => setView({ kind: 'bing', rows: await seoApi.bingStats('query', site.trim()), range: 'Bing: sorgular' }))} data-kor-cmd="seo.bingStats">Bing sorguları</button>
            <button type="button" className="btn small ghost" disabled={busy || !site.trim()} onClick={() => act(async () => setView({ kind: 'bing', rows: await seoApi.bingStats('pages', site.trim()), range: 'Bing: sayfalar' }))} data-kor-cmd="seo.bingStats">Bing sayfaları</button>
          </>
        ) : (
          <>
            <label className="kg-field">
              <span>Bing Webmaster API anahtarı</span>
              <input type="password" value={bingKey} onChange={(e) => setBingKey(e.target.value)} autoComplete="off" maxLength={128} />
            </label>
            <button type="button" className="btn small ghost" disabled={busy || !bingKey.trim()} onClick={() => act(async () => { await seoApi.bingSetup(bingKey.trim()); setBingKey(''); await bing.mutate() })} data-kor-cmd="seo.bingSetup">
              <KeyRound size={14} aria-hidden="true" />
              Kaydet
            </button>
          </>
        )}
      </div>
      {note && <p className="muted kp-small" role="status">{note}</p>}
      {busy && <p className="muted kp-small" role="status">Okunuyor…</p>}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
      {view && <ViewTable view={view} explain={explain} setExplain={setExplain} />}
    </section>
  )
}

function ViewTable({ view, explain, setExplain }: { view: View; explain: boolean; setExplain: (v: boolean) => void }) {
  if (view.kind === 'bing') {
    return view.rows.length === 0 ? <p className="ks-empty">Bing bu site için veri döndürmedi.</p> : (
      <div className="kp-table-wrap">
        <table className="kp-table">
          <caption className="kp-caption">{view.range}</caption>
          <thead><tr><th scope="col">Anahtar</th><th scope="col">Tıklama</th><th scope="col">Gösterim</th><th scope="col">Ort. konum</th></tr></thead>
          <tbody>
            {view.rows.slice(0, 200).map((r) => (
              <tr key={r.key}>
                <td translate="no">{r.key}</td>
                <td className="kp-num">{r.clicks.toFixed(0)}</td>
                <td className="kp-num">{r.impressions.toFixed(0)}</td>
                <td className="kp-num">{r.avg_impression_position.toFixed(1)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    )
  }
  if (view.kind === 'drops') {
    return view.drops.length === 0 ? (
      <p className="ks-empty">{`${view.range}: tıklaması düşen sayfa yok.`}</p>
    ) : (
      <>
        {view.alert && <p className="error-text kp-small" role="alert" translate="no">{view.alert}</p>}
        <ul className="kp-notes" aria-label={view.range} translate="no">
          {view.drops.map((d) => (
            <li key={d.page}>{`${d.page}: ${d.clicks_before.toFixed(0)} → ${d.clicks_now.toFixed(0)} tıklama, konum ${d.position_before.toFixed(1)} → ${d.position_now.toFixed(1)} · ${d.hints.map((h) => HINT[h] ?? h).join('; ')}`}</li>
          ))}
        </ul>
        {view.hypotheses && (
          <div>
            <h4 className="kp-h">Olası nedenler (model hipotezi)</h4>
            <p className="kp-pre kp-small" translate="no">{view.hypotheses}</p>
          </div>
        )}
      </>
    )
  }
  if (view.kind === 'ga') {
    const cols = view.rows.length > 0 ? Object.keys(view.rows[0]) : []
    return view.rows.length === 0 ? <p className="ks-empty">GA4 bu aralıkta satır döndürmedi.</p> : (
      <div className="kp-table-wrap">
        <table className="kp-table">
          <caption className="kp-caption">{view.range}</caption>
          <thead><tr>{cols.map((c) => <th key={c} scope="col" translate="no">{c}</th>)}</tr></thead>
          <tbody>{view.rows.slice(0, 200).map((r, i) => <tr key={i}>{cols.map((c) => <td key={c} translate="no">{String(r[c])}</td>)}</tr>)}</tbody>
        </table>
      </div>
    )
  }
  if (view.kind === 'opp') {
    return (
      <>
        <p className="muted kp-small kp-pre" translate="no">{view.formula}</p>
        {view.calibration && <p className="kp-small">{view.calibration}</p>}
        <label className="kseo-check"><input type="checkbox" checked={explain} onChange={(e) => setExplain(e.target.checked)} />Hesabı göster</label>
        <div className="kp-table-wrap">
          <table className="kp-table">
            <caption className="kp-caption">{view.range}</caption>
            <thead>
              <tr><th scope="col">Anahtar</th><th scope="col">Gösterim</th><th scope="col">TO</th><th scope="col">Konum</th><th scope="col">Ek tıklama</th><th scope="col">Endeks</th></tr>
            </thead>
            <tbody>
              {view.rows.map((o) => (
                <tr key={o.keys.join('|')}>
                  <td translate="no">
                    {o.keys.join(' · ')}
                    {explain && <div className="muted kseo-fix">{`${o.impressions.toFixed(0)} × max(0, ${o.expected_ctr.toFixed(3)} [konum ${o.target}, ${o.ctr_source === 'site' ? 'sitenin verisi' : 'varsayılan eğri'}] − ${o.ctr.toFixed(3)}) × ${o.band} = ${o.score.toFixed(1)}`}</div>}
                  </td>
                  <td className="kp-num">{o.impressions.toFixed(0)}</td>
                  <td className="kp-num">{pct(o.ctr)}</td>
                  <td className="kp-num">{o.position.toFixed(1)}</td>
                  <td className="kp-num">{o.extra_clicks.toFixed(0)}</td>
                  <td className="kp-num">{o.index.toFixed(1)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {view.rows.some((o) => o.ctr_source === 'default') && <p className="ks-note">Bazı beklenen TO değerleri Kor'un varsayılan eğrisinden gelir (bir varsayım); sitenin o konumda 5'ten az satırı var.</p>}
      </>
    )
  }
  return view.rows.length === 0 ? <p className="ks-empty">Bu aralıkta veri yok.</p> : (
    <div className="kp-table-wrap">
      <table className="kp-table">
        <caption className="kp-caption">{view.range}</caption>
        <thead><tr><th scope="col">Anahtar</th><th scope="col">Tıklama</th><th scope="col">Gösterim</th><th scope="col">TO</th><th scope="col">Konum</th></tr></thead>
        <tbody>
          {view.rows.map((r) => (
            <tr key={r.keys.join('|')}>
              <td translate="no">{r.keys.join(' · ')}</td>
              <td className="kp-num">{r.clicks.toFixed(0)}</td>
              <td className="kp-num">{r.impressions.toFixed(0)}</td>
              <td className="kp-num">{pct(r.ctr)}</td>
              <td className="kp-num">{r.position.toFixed(1)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
