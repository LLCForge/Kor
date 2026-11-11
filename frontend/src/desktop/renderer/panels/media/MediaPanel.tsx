// "Medya" paneli (§20, ADR 0060): sağlayıcılar arası yetenek kataloğu, eksik parametreleri tek kartta
// soran üretim, projenin galerisi (önizleme, türeme zinciri, "Referans yap" ve hızlı eylemler, projeye
// kaydetme) ve işler (ilerleme, iptal, önceki oturumdan kalan istekleri toplama).
// CLI: kor media models --capability | generate | gallery | show | derive | export | jobs | collect.
import { convertFileSrc } from '@tauri-apps/api/core'
import { Clipboard, Download, FolderOutput, ImagePlus, Images, ListChecks, RefreshCw, Sparkles, Terminal, Wand2, X } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { Capability, CatalogModel, GalleryItem, JsonValue, MediaPlan } from '../../../ipc-media.ts'
import { errorMessage } from '../../api.ts'
import { onNativeDrop } from '../../kor-bridge.ts'
import type { KorPanelProps } from '../registry.ts'
import { mediaApi } from './api.ts'
import { CAPABILITIES, QUICK_ACTIONS, cardFromPlan, capabilityLabel, costText, modelRef, needsReference, presetFor, priceText, promptOptional, valueText, type Preset } from './media-view.ts'
import { DirectTab } from './DirectTab.tsx'
import { ParamCardView } from './ParamCard.tsx'
import './panel.css'

type Tab = 'generate' | 'gallery' | 'catalog' | 'jobs' | 'direct'

function Preview({ path, kind, alt }: { path: string; kind: string; alt: string }) {
  const src = convertFileSrc(path)
  if (kind === 'video') return <video className="km-media" src={src} controls preload="metadata" aria-label={alt} />
  if (kind === 'audio') return <audio className="km-audio" src={src} controls preload="metadata" aria-label={alt} />
  if (kind === 'image') return <img className="km-media" src={src} alt={alt} loading="lazy" />
  return <span className="km-hint" translate="no">{path}</span>
}

// ---------------------------------------------------------------------------------------------
// Üret
// ---------------------------------------------------------------------------------------------

function GenerateTab({ preset }: { preset: Preset | null }) {
  const uid = useId()
  const [cap, setCap] = useState<Capability>(preset?.capability ?? 'image')
  const [model, setModel] = useState<string>(preset?.model ?? '')
  const [prompt, setPrompt] = useState(preset?.prompt ?? '')
  const [reference, setReference] = useState<string | null>(preset?.reference ?? null)
  const [plan, setPlan] = useState<MediaPlan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const [result, setResult] = useState<{ paths: string[]; report: string } | null>(null)
  const [saveAs, setSaveAs] = useState('')
  const catalog = useSWR(['media.catalog', cap], () => mediaApi.catalog(cap), { revalidateOnFocus: false })
  const models = catalog.data?.models ?? []

  useEffect(() => {
    if (!preset) return
    setCap(preset.capability)
    setModel(preset.model ?? '')
    setPrompt(preset.prompt)
    setReference(preset.reference)
    setPlan(null)
    setResult(null)
  }, [preset])

  useEffect(() => {
    if (!model && models[0]) setModel(modelRef(models[0]))
  }, [models, model])

  // §20.3 sürükle-bırak: işletim sisteminden bırakılan dosya galeriye referans olarak kopyalanır.
  useEffect(
    () =>
      onNativeDrop((d) => {
        const first = d.type === 'drop' ? d.paths[0] : undefined
        if (!first) return
        // Yalnızca medya panelinin üstüne bırakılanlar; başka yere bırakılan dosya buraya gelmez.
        if (!document.elementFromPoint(d.x, d.y)?.closest('[data-kor-dropzone="media"]')) return
        void mediaApi
          .importRef(first)
          .then((r) => setReference(r.id))
          .catch((e: unknown) => setError(errorMessage(e)))
      }),
    [],
  )

  useEffect(() => {
    const offProgress = mediaApi.onProgress((p) => setStatus(p.status))
    return () => offProgress()
  }, [])

  const paste = async () => {
    setError(null)
    try {
      setReference((await mediaApi.pasteRef()).id)
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const prepare = async (modelOverride?: string) => {
    const m = modelOverride ?? model
    setError(null)
    setResult(null)
    if (!m) return setError('Önce bir model seçin.')
    if (!promptOptional(cap) && !prompt.trim()) return setError('Prompt gerekli.')
    if (needsReference(cap) && !reference) return setError('Bu yetenek bir girdi dosyası ister: galeriden "Referans yap", yapıştır ya da sürükle.')
    setBusy(true)
    try {
      setPlan(await mediaApi.plan(cap, m, prompt, preset?.params ?? {}))
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const run = async (values: Record<string, JsonValue>) => {
    if (!plan) return
    setBusy(true)
    setError(null)
    setStatus('Kuyruğa alınıyor…')
    try {
      const params = { ...plan.plan.input, ...values }
      const r = await mediaApi.generate(cap, modelRef(plan.model), prompt, params, reference ?? undefined, {
        saveAs: saveAs.trim() || undefined,
        action: preset?.action ?? undefined,
        derivedFrom: preset?.derivedFrom ?? undefined,
      })
      setResult({ paths: r.paths, report: r.report })
      setPlan(null)
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
      setStatus(null)
    }
  }

  return (
    <section className="km-stack" aria-labelledby={`${uid}-h`} data-kor-event="kor:media/progress kor:media/done">
      <h3 id={`${uid}-h`} className="km-h">Üret</h3>
      <div className="km-chips" role="radiogroup" aria-label="Yetenek">
        {CAPABILITIES.map((c) => (
          <button key={c.id} type="button" role="radio" aria-checked={cap === c.id} className={`km-chip${cap === c.id ? ' on' : ''}`} onClick={() => { setCap(c.id); setModel(''); setPlan(null) }}>
            {c.label}
          </button>
        ))}
      </div>
      <label className="km-field">
        <span className="km-field-name">Model</span>
        <select value={model} onChange={(e) => { setModel(e.target.value); setPlan(null) }} data-kor-cmd="media.catalog">
          {models.length === 0 && <option value="">{catalog.isLoading ? 'Yükleniyor…' : 'Bu yetenekte model yok (anahtar?)'}</option>}
          {preset?.model && !models.some((m) => modelRef(m) === preset.model) && <option value={preset.model}>{preset.model}</option>}
          {models.map((m) => (
            <option key={modelRef(m)} value={modelRef(m)}>
              {modelRef(m)} — {priceText(m.price)}
            </option>
          ))}
        </select>
      </label>
      {catalog.error && <p className="error-text">{errorMessage(catalog.error)}</p>}
      <label className="km-field">
        <span className="km-field-name">Prompt{promptOptional(cap) ? ' (isteğe bağlı)' : ''}</span>
        <textarea rows={3} value={prompt} onChange={(e) => { setPrompt(e.target.value); setPlan(null) }} placeholder="Örn. bir kedi videosu, 16:9" />
      </label>
      <div className="km-row">
        <span className="km-field-name">Referans:</span>
        {reference ? (
          <span className="badge neutral" translate="no">{reference}</span>
        ) : (
          <span className="km-hint">yok (galeriden seçin, yapıştırın ya da dosya sürükleyin)</span>
        )}
        {reference && (
          <button type="button" className="action-btn" aria-label="Referansı kaldır" title="Referansı kaldır" onClick={() => setReference(null)}>
            <X size={14} aria-hidden="true" />
          </button>
        )}
        <button type="button" className="btn small" onClick={() => void paste()} data-kor-cmd="media.pasteRef">
          <Clipboard size={14} aria-hidden="true" /> Panodan yapıştır
        </button>
        <span className="km-hint" data-kor-cmd="media.importRef">Sürükle-bırak desteklenir</span>
      </div>
      <label className="km-field">
        <span className="km-field-name">Projeye de kaydet (isteğe bağlı)</span>
        <input value={saveAs} onChange={(e) => setSaveAs(e.target.value)} placeholder="public/logo.png" />
      </label>
      {!plan && (
        <div className="km-actions">
          <button type="button" className="btn primary" disabled={busy} onClick={() => void prepare()} data-kor-cmd="media.plan">
            <Wand2 size={14} aria-hidden="true" /> Parametreleri hazırla
          </button>
        </div>
      )}
      {plan && (
        <>
          {plan.alternatives.filter((a) => a.models.length > 0).map((a) => (
            <div key={a.name} className="km-alts">
              <span className="km-hint"><span translate="no">{a.name}</span> için bunu yapabilen modeller (ucuzdan pahalıya):</span>
              {a.models.map((x) => (
                <button key={modelRef(x.model)} type="button" className="btn small" onClick={() => { setModel(modelRef(x.model)); void prepare(modelRef(x.model)) }} data-kor-cmd="media.plan">
                  <span translate="no">{modelRef(x.model)}</span> {x.estimate ? `${x.estimate.estimated ? '≈' : ''}$${x.estimate.amount.toFixed(2)}` : '?'}
                </button>
              ))}
            </div>
          ))}
          <ParamCardView key={modelRef(plan.model) + prompt} card={cardFromPlan(plan, prompt)} busy={busy} onSubmit={(v) => void run(v)} onCancel={() => setPlan(null)} cmd="media.generate" />
          <span hidden data-kor-cmd="media.generate" />
        </>
      )}
      {status && <p className="km-hint" role="status">{status}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      {result && (
        <div className="km-result">
          {result.paths.map((p) => (
            <Preview key={p} path={p} kind={/\.(mp4|webm|mov)$/i.test(p) ? 'video' : /\.(mp3|wav|ogg|flac|m4a)$/i.test(p) ? 'audio' : /\.(png|jpe?g|webp|gif)$/i.test(p) ? 'image' : 'other'} alt={p} />
          ))}
          <pre className="km-pre" translate="no">{result.report}</pre>
        </div>
      )}
    </section>
  )
}

// ---------------------------------------------------------------------------------------------
// Galeri
// ---------------------------------------------------------------------------------------------

function Detail({ id, onPreset, onClose }: { id: string; onPreset: (p: Preset) => void; onClose: () => void }) {
  const detail = useSWR(['media.item', id], () => mediaApi.item(id), { revalidateOnFocus: false })
  const [dest, setDest] = useState('')
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)
  const d = detail.data
  const save = async () => {
    setMsg(null)
    try {
      const r = await mediaApi.export(id, dest.trim())
      setMsg({ ok: true, text: `Kaydedildi: ${r.path}` })
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    }
  }
  if (detail.error) return <p className="error-text">{errorMessage(detail.error)}</p>
  if (!d) return <p className="km-hint">Yükleniyor…</p>
  const it = d.item
  return (
    <div className="km-detail" data-kor-cmd="media.item">
      <div className="km-row">
        <h4 className="km-h4" translate="no">{it.id}</h4>
        <span className="km-grow" />
        <button type="button" className="action-btn" aria-label="Ayrıntıyı kapat" title="Kapat" onClick={onClose}>
          <X size={14} aria-hidden="true" />
        </button>
      </div>
      <Preview path={d.path} kind={it.kind} alt={it.prompt || it.id} />
      <dl className="km-facts">
        <dt>Model</dt>
        <dd translate="no">{it.provider ? `${it.provider}:${it.model}` : '—'}</dd>
        <dt>Yetenek</dt>
        <dd>{capabilityLabel(it.capability)}</dd>
        {it.prompt && (<><dt>Prompt</dt><dd translate="no">{it.prompt}</dd></>)}
        {it.seed !== null && (<><dt>Seed</dt><dd translate="no">{it.seed}</dd></>)}
        {it.cost !== null && (<><dt>Maliyet</dt><dd translate="no">{costText(it)}</dd></>)}
        {it.secs > 0 && (<><dt>Süre</dt><dd>{it.secs} sn</dd></>)}
        {it.action && (<><dt>Eylem</dt><dd translate="no">{it.action}</dd></>)}
        <dt>Parametreler</dt>
        <dd><code className="km-code" translate="no">{valueText(it.params)}</code></dd>
      </dl>
      <div className="km-actions km-wrap">
        <button type="button" className="btn small primary" onClick={() => onPreset(presetFor(it, 'reference'))}>
          <ImagePlus size={14} aria-hidden="true" /> Referans yap
        </button>
        {QUICK_ACTIONS.map((a) => (
          <button key={a.id} type="button" className="btn small" onClick={() => onPreset(presetFor(it, a.id))}>
            {a.label}
          </button>
        ))}
      </div>
      <div className="km-row">
        <input className="km-grow" value={dest} onChange={(e) => setDest(e.target.value)} placeholder="public/logo.png" aria-label="Projedeki hedef yol" />
        <button type="button" className="btn small" disabled={!dest.trim()} onClick={() => void save()} data-kor-cmd="media.export">
          <FolderOutput size={14} aria-hidden="true" /> Projeye kaydet
        </button>
      </div>
      {msg && <p className={msg.ok ? 'km-ok' : 'error-text'} role="status">{msg.text}</p>}
      {d.chain.length > 0 && (
        <>
          <h4 className="km-h4">Türediği zincir</h4>
          <ol className="km-chain">
            {d.chain.map((c) => (
              <li key={c.item.id}>
                <Preview path={c.path} kind={c.item.kind} alt={c.item.id} />
                <span translate="no">{c.item.id}</span>
                {c.item.action && <span className="badge neutral" translate="no">{c.item.action}</span>}
              </li>
            ))}
          </ol>
        </>
      )}
      {d.children.length > 0 && (
        <>
          <h4 className="km-h4">Bundan türeyenler</h4>
          <ul className="km-list">
            {d.children.map((c) => (
              <li key={c.id} translate="no">{c.id}{c.action ? ` [${c.action}]` : ''}</li>
            ))}
          </ul>
        </>
      )}
    </div>
  )
}

function GalleryTab({ onPreset }: { onPreset: (p: Preset) => void }) {
  const gallery = useSWR('media.gallery', () => mediaApi.gallery(), { revalidateOnFocus: false })
  const [selected, setSelected] = useState<string | null>(null)
  const { mutate } = gallery
  useEffect(() => mediaApi.onDone(() => void mutate()), [mutate])
  const items: GalleryItem[] = gallery.data ?? []
  return (
    <section className="km-stack" data-kor-event="kor:media/done">
      <div className="km-row">
        <h3 className="km-h">Galeri</h3>
        <span className="km-grow" />
        <button type="button" className="action-btn" aria-label="Galeriyi yenile" title="Yenile" onClick={() => void mutate()} data-kor-cmd="media.gallery">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {gallery.error && <p className="error-text">{errorMessage(gallery.error)}</p>}
      {gallery.data && items.length === 0 && <p className="km-empty">Bu projede henüz üretilmiş medya yok. Üret sekmesinden ya da sohbetten üretilenler burada görünür.</p>}
      {selected && <Detail key={selected} id={selected} onPreset={onPreset} onClose={() => setSelected(null)} />}
      <ul className="km-grid" data-kor-cmd="media.gallery">
        {items.map((it) => (
          <li key={it.id}>
            <button type="button" className={`km-tile${selected === it.id ? ' on' : ''}`} onClick={() => setSelected(it.id)} aria-label={`${it.id} ayrıntısı`}>
              {it.path && it.kind === 'image' ? <img src={convertFileSrc(it.path)} alt="" loading="lazy" /> : <span className="km-tile-kind">{it.kind}</span>}
              <span className="km-tile-name" translate="no">{it.id}</span>
              {it.parent && <span className="km-hint" translate="no">← {it.parent}</span>}
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}

// ---------------------------------------------------------------------------------------------
// Katalog
// ---------------------------------------------------------------------------------------------

function CatalogTab() {
  const [cap, setCap] = useState<Capability | ''>('')
  const catalog = useSWR(['media.catalog', cap || 'all'], () => mediaApi.catalog(cap || undefined), { revalidateOnFocus: false })
  const [busy, setBusy] = useState(false)
  const refresh = async () => {
    setBusy(true)
    try {
      await catalog.mutate(await mediaApi.catalog(cap || undefined, true), { revalidate: false })
    } finally {
      setBusy(false)
    }
  }
  const data = catalog.data
  return (
    <section className="km-stack">
      <div className="km-row">
        <h3 className="km-h">Yetenek kataloğu</h3>
        <span className="km-grow" />
        <button type="button" className="btn small" disabled={busy} onClick={() => void refresh()} data-kor-cmd="media.catalog">
          <RefreshCw size={14} aria-hidden="true" /> Şimdi güncelle
        </button>
      </div>
      <div className="km-chips" role="radiogroup" aria-label="Yetenek süzgeci">
        <button type="button" role="radio" aria-checked={cap === ''} className={`km-chip${cap === '' ? ' on' : ''}`} onClick={() => setCap('')}>Hepsi</button>
        {CAPABILITIES.map((c) => (
          <button key={c.id} type="button" role="radio" aria-checked={cap === c.id} className={`km-chip${cap === c.id ? ' on' : ''}`} onClick={() => setCap(c.id)}>
            {c.label}
          </button>
        ))}
      </div>
      {catalog.isLoading && <p className="km-hint">Katalog yükleniyor…</p>}
      {catalog.error && <p className="error-text">{errorMessage(catalog.error)}</p>}
      {data && (
        <p className="km-hint">
          {data.providers.length === 0
            ? 'Medya sağlayıcısı yapılandırılmamış (FAL_KEY, Venice anahtarı ya da HF_TOKEN) ya da gizlilik modu açık.'
            : `Sağlayıcılar: ${data.providers.join(', ')} · ${data.fetched ? new Date(data.fetched).toLocaleString(uiLocale()) : ''} · ≈ tahmini`}
        </p>
      )}
      {data && data.errors.length > 0 && <p className="km-warn" translate="no">{data.errors.join(' · ')}</p>}
      {data && data.providers.length > 0 && data.models.length === 0 && <p className="km-empty">Bu yetenekte model bulunamadı.</p>}
      <ul className="km-list">
        {(data?.models ?? []).map((m: CatalogModel) => (
          <li key={modelRef(m)} className="km-model">
            <div className="km-row">
              <span className="km-name" translate="no">{modelRef(m)}</span>
              <span className="km-grow" />
              <span className="badge neutral" translate="no">{priceText(m.price)}</span>
            </div>
            <div className="km-row km-wrap">
              {m.capabilities.map((c) => <span key={c} className="km-tag">{capabilityLabel(c)}</span>)}
            </div>
            <ul className="km-params">
              {m.params.map((p) => (
                <li key={p.name} translate="no">
                  <b>{p.name}{p.required ? '*' : ''}</b> <span className="km-hint">{p.kind}</span>
                  {p.options && p.options.length > 0 && <> [{p.options.slice(0, 8).map(valueText).join(' | ')}{p.options.length > 8 ? ' | …' : ''}]</>}
                  {(p.min !== undefined || p.max !== undefined) && <> ({p.min ?? ''}–{p.max ?? ''})</>}
                  {p.default !== undefined && <> = {valueText(p.default)}</>}
                </li>
              ))}
            </ul>
            <span className="km-hint" translate="no">Kaynak: {m.source}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

// ---------------------------------------------------------------------------------------------
// İşler
// ---------------------------------------------------------------------------------------------

function JobsTab() {
  const running = useSWR('media.jobs', () => mediaApi.jobs(), { refreshInterval: (d) => (d && d.length > 0 ? 2000 : 0), revalidateOnFocus: false })
  const pending = useSWR('media.pending', () => mediaApi.pending(), { revalidateOnFocus: false })
  const [lines, setLines] = useState<Record<string, string>>({})
  const [msg, setMsg] = useState<string | null>(null)
  const [collecting, setCollecting] = useState<string | null>(null)
  const mutateRunning = running.mutate
  const mutatePending = pending.mutate
  useEffect(() => {
    const offs = [
      mediaApi.onProgress((p) => {
        setLines((l) => ({ ...l, [p.jobId]: p.status }))
        void mutateRunning()
      }),
      mediaApi.onDone(() => {
        void mutateRunning()
        void mutatePending()
      }),
      mediaApi.onCancelled(() => void mutateRunning()),
    ]
    return () => offs.forEach((off) => off())
  }, [mutateRunning, mutatePending])
  const cancel = async (id: string) => {
    setMsg(null)
    try {
      await mediaApi.cancel(id)
    } catch (e) {
      setMsg(errorMessage(e))
    }
  }
  const collect = async (id?: string) => {
    setMsg(null)
    setCollecting(id ?? 'all')
    try {
      const r = await mediaApi.collect(id)
      setMsg(`${r.collected.length} istek toplandı.`)
    } catch (e) {
      setMsg(errorMessage(e))
    } finally {
      setCollecting(null)
      void mutatePending()
    }
  }
  const run = running.data ?? []
  const wait = pending.data ?? []
  return (
    <section className="km-stack" data-kor-event="kor:media/progress kor:media/done kor:media/cancelled">
      <h3 className="km-h">Çalışan işler</h3>
      {running.data && run.length === 0 && <p className="km-empty">Çalışan üretim yok.</p>}
      <ul className="km-list" data-kor-cmd="media.jobs">
        {run.map((j) => (
          <li key={j.jobId} className="km-row">
            <span className="badge neutral">{j.kind === 'hf' ? 'İndirme' : 'Üretim'}</span>
            <span className="km-name" translate="no">{j.target}</span>
            <span className="km-hint" translate="no">{lines[j.jobId] ?? ''}</span>
            <span className="km-grow" />
            <button type="button" className="action-btn" aria-label={`${j.target} işini iptal et`} title="İptal et" onClick={() => void cancel(j.jobId)} data-kor-cmd="media.cancel">
              <X size={14} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
      <div className="km-row">
        <h3 className="km-h">Toplanmayı bekleyen istekler</h3>
        <span className="km-grow" />
        {wait.length > 1 && (
          <button type="button" className="btn small" disabled={collecting !== null} onClick={() => void collect()} data-kor-cmd="media.collect">
            <Download size={14} aria-hidden="true" /> Hepsini topla
          </button>
        )}
      </div>
      <p className="km-hint">Kor kapansa da sağlayıcıda süren işler (video gibi) burada kalır; bittiğinde toplanıp galeriye eklenir.</p>
      {pending.data && wait.length === 0 && <p className="km-empty">Bekleyen istek yok.</p>}
      <ul className="km-list" data-kor-cmd="media.pending">
        {wait.map((j) => (
          <li key={j.id} className="km-row">
            <span className="badge neutral" translate="no">{j.provider}</span>
            <span className="km-name" translate="no">{j.model}</span>
            <span className="km-hint" translate="no">{j.prompt.slice(0, 60)}</span>
            <span className="km-grow" />
            <button type="button" className="btn small" disabled={collecting !== null} onClick={() => void collect(j.id)} data-kor-cmd="media.collect">
              {collecting === j.id ? 'Bekleniyor…' : 'Topla'}
            </button>
          </li>
        ))}
      </ul>
      {msg && <p className="km-hint" role="status">{msg}</p>}
    </section>
  )
}

// ---------------------------------------------------------------------------------------------

const TABS: readonly { id: Tab; label: string; icon: typeof Sparkles }[] = [
  { id: 'generate', label: 'Üret', icon: Sparkles },
  { id: 'gallery', label: 'Galeri', icon: Images },
  { id: 'catalog', label: 'Katalog', icon: ListChecks },
  { id: 'jobs', label: 'İşler', icon: RefreshCw },
  { id: 'direct', label: 'Doğrudan', icon: Terminal },
]

export function MediaPanel(_props: KorPanelProps) {
  const [tab, setTab] = useState<Tab>('generate')
  const [preset, setPreset] = useState<Preset | null>(null)
  const applyPreset = (p: Preset) => {
    setPreset(p)
    setTab('generate')
  }
  return (
    <div className="km-panel" data-kor-dropzone="media">
      <div className="km-tabs" role="tablist" aria-label="Medya">
        {TABS.map((t) => (
          <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className={`km-tab${tab === t.id ? ' on' : ''}`} onClick={() => setTab(t.id)}>
            <t.icon size={14} aria-hidden="true" /> {t.label}
          </button>
        ))}
      </div>
      <div role="tabpanel">
        {tab === 'generate' && <GenerateTab preset={preset} />}
        {tab === 'gallery' && <GalleryTab onPreset={applyPreset} />}
        {tab === 'catalog' && <CatalogTab />}
        {tab === 'jobs' && <JobsTab />}
        {tab === 'direct' && <DirectTab />}
      </div>
    </div>
  )
}

import { uiLocale } from '../../ui-locale.ts'