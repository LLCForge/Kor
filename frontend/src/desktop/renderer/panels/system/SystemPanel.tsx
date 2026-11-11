// Sistem görünümü (§19, §27.7, ADR 0059): bu makinenin profili (ajanın gördüğü), sağlık denetimi,
// onaylı önbellek temizliği, masaüstü denetimi ve uygulama katmanları, pencere düzenleri.
import { Archive, BellOff, Brush, Cpu, HardDrive, LayoutGrid, MonitorCog, Play, RefreshCw, RotateCcw, Save, ShieldCheck, Square, Trash2 } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { CleanCandidate, CleanOutcome, ComputerStatus, ExportSummary, ComputerTier, HealthAlert, SystemProfile } from '../../../ipc-system.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { CLEAN_EVENT, computerApi, layoutApi, systemApi } from './bridge.ts'
import { CLASS_LABEL, TIERS, TIER_LABEL, bytes, diskUsed, mb, validApp, validLayout } from './shape.ts'

function useAction() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true)
    setError(null)
    try {
      return await fn()
    } catch (err) {
      setError(errorMessage(err))
      return undefined
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, run }
}

function ProfileCard({ p }: { p: SystemProfile }) {
  const id = useId()
  const age = Math.max(0, Math.round((Date.now() / 1000 - p.collected_at) / 60))
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><Cpu size={16} aria-hidden="true" />Bu makine</h3>
      <p className="muted ks-note">Ajan bu bilgileri bağlam olarak görür: size işletim sistemini sormaz, doğru paket yöneticisini ve yolları kullanır. {age} dk önce çıkarıldı.</p>
      <dl className="ks-facts" translate="no">
        <dt>İşletim sistemi</dt>
        <dd>{p.os.long_name || p.os.name} {p.os.arch}{p.os.distro ? ` · ${p.os.distro.name} ${p.os.distro.version}${p.os.distro.immutable ? ' (değişmez)' : ''}` : ''}</dd>
        {p.os.desktop && (<><dt>Masaüstü</dt><dd>{p.os.desktop}</dd></>)}
        <dt>Paket yöneticileri</dt>
        <dd>{p.os.package_managers.join(', ') || '—'}</dd>
        <dt>İşlemci</dt>
        <dd>{p.cpu.model} ({p.cpu.threads} iş parçacığı)</dd>
        <dt>Bellek</dt>
        <dd>{mb(p.memory.total_mb)} ({mb(p.memory.available_mb)} boş)</dd>
        <dt>Ekran kartı</dt>
        <dd>{p.gpus.map((g) => `${g.name}${g.vram_mb != null ? ` · ${mb(g.vram_mb)} VRAM` : ''}`).join(', ') || '—'}</dd>
        <dt>Diskler</dt>
        <dd>
          {p.disks.map((d) => (
            <span key={d.mount} className="ksys-disk">
              {d.mount} {mb(d.free_mb)} boş / {mb(d.total_mb)}
              <meter min={0} max={100} low={85} high={95} optimum={50} value={diskUsed(d)} aria-label={`${d.mount} doluluk yüzdesi`} />
            </span>
          ))}
        </dd>
        {p.battery && (<><dt>Pil</dt><dd>%{p.battery.percent}{p.battery.charging ? ', şarj oluyor' : ''}</dd></>)}
        <dt>Araçlar</dt>
        <dd>{Object.entries(p.toolchains).map(([k, v]) => `${k} ${v}`).join(', ') || '—'}</dd>
        <dt>Kabuk / terminal</dt>
        <dd>{p.shell ?? '—'} / {p.terminal ?? '—'}</dd>
        <dt>Ekranlar</dt>
        <dd>{p.displays.map((d) => `${d.width}×${d.height}${d.primary ? ' (birincil)' : ''}`).join(', ') || '—'}</dd>
        <dt>Klavye</dt>
        <dd>{p.keyboard ? `${p.keyboard.lang ?? p.keyboard.raw}${p.keyboard.variant ? ` (${p.keyboard.variant})` : ''}` : '—'}</dd>
        <dt>Ağ</dt>
        <dd>{p.network.map((n) => n.name).join(', ') || '—'}</dd>
      </dl>
    </section>
  )
}

function HealthCard({ onClean }: { onClean: () => void }) {
  const id = useId()
  const [alerts, setAlerts] = useState<HealthAlert[] | null>(null)
  const [deep, setDeep] = useState(false)
  const { busy, error, run } = useAction()
  const check = async () => {
    const a = await run(() => systemApi.health(deep))
    if (a) setAlerts(a)
  }
  const mute = async (kind: string) => {
    const ok = await confirmAction({ title: 'Bu uyarı türü sessize alınsın mı?', message: `"${kind}" uyarıları bir daha gösterilmez. Ayarlardan (health.muted) geri açabilirsiniz.`, confirmLabel: 'Sessize al' })
    if (ok && (await run(() => systemApi.mute(kind)))) setAlerts((a) => (a ? a.filter((x) => x.kind !== kind) : a))
  }
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><ShieldCheck size={16} aria-hidden="true" />Sağlık</h3>
      <p className="muted ks-note">Disk doluluğu (projenin diski önce), bellek ve takas baskısı, Kor'un ve başlattığı süreçlerin yükü, kaçak süreçler, işlemci sıcaklığı, pil, derleme önbellekleri, Docker ve Kor'un kendi önbelleği. Her şey yereldir; disk kritik doluysa Kor yazma gerektiren adımları başlamadan durdurur.</p>
      <label className="ks-check">
        <input type="checkbox" checked={deep} onChange={(e) => setDeep(e.target.checked)} />
        Önbellekleri, Docker'ı ve Kor önbelleğini de ölç (daha yavaş)
      </label>
      <div className="ko-actions">
        <button type="button" className="btn small" disabled={busy} onClick={() => void check()} data-kor-cmd="system.health">
          <RefreshCw size={14} aria-hidden="true" />{busy ? 'Denetleniyor…' : 'Şimdi denetle'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {alerts && alerts.length === 0 && <p className="ks-ok">Her şey yolunda: uyarı yok.</p>}
      {alerts && alerts.length > 0 && (
        <ul className="ksys-list">
          {alerts.map((a) => (
            <li key={a.id} className={`ksys-alert ${a.level}`}>
              <strong>{a.title}</strong>
              {a.body && <span className="muted" translate="no">{a.body}</span>}
              <span className="ko-actions">
                {a.clean && (
                  <button type="button" className="btn small" onClick={onClean}>
                    <Brush size={14} aria-hidden="true" />Temizlik önerileri
                  </button>
                )}
                <button type="button" className="action-btn" aria-label={`${a.title} uyarısını sessize al`} title="Sessize al" onClick={() => void mute(a.kind)} data-kor-cmd="system.mute">
                  <BellOff size={14} aria-hidden="true" />
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function CleanCard({ open }: { open: number }) {
  const id = useId()
  const [items, setItems] = useState<CleanCandidate[] | null>(null)
  const [chosen, setChosen] = useState<string[]>([])
  const [done, setDone] = useState<CleanOutcome[] | null>(null)
  const { busy, error, run } = useAction()
  const load = async () => {
    setDone(null)
    const list = await run(() => systemApi.cleanSuggest())
    if (list) {
      setItems(list)
      setChosen([])
    }
  }
  useEffect(() => {
    if (open > 0) void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])
  const total = (items ?? []).filter((c) => chosen.includes(c.id)).reduce((s, c) => s + c.bytes, 0)
  const clean = async () => {
    const ok = await confirmAction({
      title: 'Seçilenler silinsin mi?',
      message: `${chosen.length} öğe (${bytes(total)}) kalıcı olarak silinecek. Yalnızca yeniden oluşturulabilen önbellek klasörleri silinir; Docker için yalnızca sahipsiz imajlar ve derleme önbelleği temizlenir.`,
      confirmLabel: 'Sil',
      danger: true,
    })
    if (!ok) return
    const out = await run(() => systemApi.clean(chosen))
    if (out) {
      setDone(out)
      setItems((list) => (list ? list.filter((c) => !out.some((o) => o.id === c.id && o.error === null)) : list))
      setChosen([])
    }
  }
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><HardDrive size={16} aria-hidden="true" />Temizlik</h3>
      <p className="muted ks-note">`kor clean --suggest` ile aynı liste: projedeki target/ ve node_modules gibi derleme önbellekleri, paket yöneticilerinin indirme önbellekleri, Docker. Onayınız olmadan hiçbir şey silinmez.</p>
      <div className="ko-actions">
        <button type="button" className="btn small" disabled={busy} onClick={() => void load()} data-kor-cmd="system.cleanSuggest">
          <RefreshCw size={14} aria-hidden="true" />{busy && !items ? 'Ölçülüyor…' : 'Önerileri getir'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {items && items.length === 0 && <p className="ks-empty">Temizlenecek bir şey yok.</p>}
      {items && items.length > 0 && (
        <fieldset className="ko-models">
          <legend>Silinebilecekler</legend>
          {items.map((c) => (
            <label key={c.id} className="ks-check">
              <input type="checkbox" checked={chosen.includes(c.id)} onChange={(e) => setChosen((x) => (e.target.checked ? [...x, c.id] : x.filter((y) => y !== c.id)))} />
              <span translate="no">{c.approx ? '≥ ' : ''}{bytes(c.bytes)} · {c.kind} · {c.path ?? 'Docker'}</span>
            </label>
          ))}
          <div className="ko-actions">
            <button type="button" className="btn small danger" disabled={busy || chosen.length === 0} onClick={() => void clean()} data-kor-cmd="system.clean">
              <Trash2 size={14} aria-hidden="true" />Seçilenleri temizle ({bytes(total)})
            </button>
          </div>
        </fieldset>
      )}
      {done && (
        <ul className="ksys-list" aria-live="polite">
          {done.map((o) => (
            <li key={o.id} className={o.error ? 'error-text' : 'ks-ok'} translate="no">
              {o.error ? `${o.path ?? 'Docker'}: ${o.error}` : `${o.path ?? 'Docker'}: ${bytes(o.freed)} açıldı`}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function ComputerCard() {
  const id = useId()
  const { data, error: loadError, mutate } = useSWR<ComputerStatus>('computer:status', () => computerApi.status())
  const [app, setApp] = useState('')
  const [tier, setTier] = useState<ComputerTier>('ask')
  const { busy, error, run } = useAction()
  useEffect(() => computerApi.onActivity(() => void mutate()), [mutate])
  const apply = async (fn: () => Promise<ComputerStatus>) => {
    const s = await run(fn)
    if (s) void mutate(s, { revalidate: false })
  }
  if (loadError) return <p className="error-text" role="alert">{errorMessage(loadError)}</p>
  if (!data) return <p className="muted" role="status">Masaüstü denetimi okunuyor…</p>
  return (
    <section className={`kg-card${data.enabled ? ' ks-privacy-on' : ''}`} aria-labelledby={`${id}-t`} data-kor-cmd="computer.status" data-kor-event="kor:computer/activity">
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <MonitorCog size={16} aria-hidden="true" />Bilgisayar kullanımı
        <span className={`ks-badge${data.enabled ? ' on' : ''}`}>{data.enabled ? 'Açık' : 'Kapalı'}</span>
      </h3>
      <label className="ks-check">
        <input type="checkbox" checked={data.enabled} disabled={busy} onChange={(e) => void apply(() => computerApi.setEnabled(e.target.checked))} data-kor-cmd="computer.setEnabled" />
        Modeller ekranı görebilir, tıklayabilir ve yazabilir (uygulama katmanlarına göre)
      </label>
      <p className="muted ks-note">
        {data.watching
          ? 'Fareyi oynattığınız ya da bir tuşa bastığınız anda Kor durur; devam etmek için onayınızı ister.'
          : 'Bu masaüstü girdinizi bildirmiyor: Kor her adımda onay ister.'}
      </p>
      {data.paused && (
        <p className="ks-warn">Kor duraklatıldı ({data.paused === 'mouse' ? 'fare hareketi' : 'tuş/girdi'}); sonraki masaüstü adımı onayınızı isteyecek.</p>
      )}
      <div className="ko-actions">
        <button type="button" className="btn small" disabled={busy} onClick={() => void apply(() => computerApi.stop())} data-kor-cmd="computer.stop">
          <Square size={14} aria-hidden="true" />Kontrolü durdur
        </button>
      </div>
      {data.front && (
        <p className="ks-note" translate="no">Öndeki uygulama: {data.front.app} · {TIER_LABEL[data.front.tier]}</p>
      )}
      <dl className="ks-facts">
        {data.classes.map((c) => (
          <div key={c.class} className="ksys-row">
            <dt>{CLASS_LABEL[c.class]}</dt>
            <dd>{TIER_LABEL[c.tier]}</dd>
          </div>
        ))}
      </dl>
      {data.apps.length > 0 && (
        <ul className="ksys-list">
          {data.apps.map((r) => (
            <li key={r.app} className="ksys-tier">
              <span translate="no">{r.app}</span>
              <select className="g7-select" aria-label={`${r.app} katmanı`} value={r.tier} disabled={busy} onChange={(e) => void apply(() => computerApi.setTier(r.app, e.target.value as ComputerTier))} data-kor-cmd="computer.setTier">
                {TIERS.map((t) => <option key={t} value={t}>{TIER_LABEL[t]}</option>)}
              </select>
              <button type="button" className="action-btn" aria-label={`${r.app} için yerleşik katmana dön`} title="Yerleşik katmana dön" disabled={busy} onClick={() => void apply(() => computerApi.resetTier(r.app))} data-kor-cmd="computer.resetTier">
                <RotateCcw size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form className="ksys-tier" onSubmit={(e) => { e.preventDefault(); if (validApp(app)) void apply(() => computerApi.setTier(app.trim(), tier)).then(() => setApp('')) }}>
        <input className="text-input" value={app} onChange={(e) => setApp(e.target.value)} placeholder="Uygulama adı (ör. mspaint)" aria-label="Uygulama adı" />
        <select className="g7-select" aria-label="Katman" value={tier} onChange={(e) => setTier(e.target.value as ComputerTier)}>
          {TIERS.map((t) => <option key={t} value={t}>{TIER_LABEL[t]}</option>)}
        </select>
        <button type="submit" className="btn small" disabled={busy || !validApp(app)} data-kor-cmd="computer.setTier">Ekle</button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function LayoutCard() {
  const id = useId()
  const { data, error: loadError, mutate } = useSWR('layout:list', () => layoutApi.list())
  const [name, setName] = useState('')
  const [link, setLink] = useState(true)
  const [note, setNote] = useState<string | null>(null)
  const { busy, error, run } = useAction()
  useEffect(() => layoutApi.onRestored((e) => setNote(`${e.name} düzeni proje açılınca geri getirildi (${e.plan.moves.length} pencere).`)), [])
  const save = async () => {
    const l = await run(() => layoutApi.save(name, link))
    if (l) {
      setNote(`${l.name}: ${l.monitors.length} monitörde ${l.windows.length} pencere kaydedildi.`)
      setName('')
      void mutate()
    }
  }
  const restore = async (n: string) => {
    const p = await run(() => layoutApi.restore(n))
    if (p) setNote(`${n}: ${p.moves.length} pencere yerleşti${p.missing.length ? `; geri getirilemeyen: ${p.missing.join(', ')}` : ''}.`)
  }
  const remove = async (n: string) => {
    if (!(await confirmAction({ title: 'Düzen silinsin mi?', message: `${n} düzeni silinecek.`, confirmLabel: 'Sil', danger: true }))) return
    if ((await run(() => layoutApi.delete(n))) !== undefined) void mutate()
  }
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:layout/restored">
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><LayoutGrid size={16} aria-hidden="true" />Pencere düzenleri</h3>
      <p className="muted ks-note">Açık pencerelerin monitörlerdeki yerleşimini kaydedin; projeye bağlı düzen proje açılınca geri gelir (`kor layout`).</p>
      <form className="ksys-tier" onSubmit={(e) => { e.preventDefault(); if (validLayout(name)) void save() }}>
        <input className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Düzen adı (ör. kodlama)" aria-label="Düzen adı" />
        <label className="ks-check"><input type="checkbox" checked={link} onChange={(e) => setLink(e.target.checked)} />Projeye bağla</label>
        <button type="submit" className="btn small" disabled={busy || !validLayout(name)} data-kor-cmd="layout.save"><Save size={14} aria-hidden="true" />Kaydet</button>
      </form>
      {loadError && <p className="error-text" role="alert">{errorMessage(loadError)}</p>}
      {data && data.layouts.length === 0 && <p className="ks-empty">Kayıtlı düzen yok.</p>}
      {data && data.layouts.length > 0 && (
        <ul className="ksys-list" data-kor-cmd="layout.list">
          {data.layouts.map((l) => (
            <li key={l.name} className="ksys-tier">
              <span translate="no">{l.name}{data.linked === l.name ? ' · projeye bağlı' : ''}</span>
              <span className="muted">{l.windows.length} pencere, {l.monitors.length} monitör</span>
              <button type="button" className="action-btn" aria-label={`${l.name} düzenini geri getir`} title="Geri getir" disabled={busy} onClick={() => void restore(l.name)} data-kor-cmd="layout.restore"><Play size={14} aria-hidden="true" /></button>
              <button type="button" className="action-btn" aria-label={`${l.name} düzenini sil`} title="Sil" disabled={busy} onClick={() => void remove(l.name)} data-kor-cmd="layout.delete"><Trash2 size={14} aria-hidden="true" /></button>
            </li>
          ))}
        </ul>
      )}
      {note && <p className="ks-note" role="status">{note}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

/** `kor export --all`: bütün kullanıcı verisi tek arşivde (§22.2); gizli anahtarlar dahil edilmez. */
function ExportCard() {
  const id = useId()
  const { busy, error, run } = useAction()
  const [done, setDone] = useState<ExportSummary | null>(null)
  const exportAll = async () => {
    const out = await run(() => systemApi.exportAll())
    if (out) setDone(out)
  }
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><Archive size={16} aria-hidden="true" />Verilerimi dışa aktar</h3>
      <p className="muted ks-note">Oturumlar, hafıza, bilgi tabanı, denetim kaydı ve ayarlar düz Markdown/JSON/SQLite olarak tek bir zip dosyasına yazılır (`kor export --all`). Gizli anahtarlar işletim sisteminin anahtarlığında kalır, arşive girmez.</p>
      <div className="ko-actions">
        <button type="button" className="btn small" disabled={busy} onClick={() => void exportAll()} data-kor-cmd="system.exportAll">
          <Archive size={14} aria-hidden="true" />{busy ? 'Arşivleniyor…' : 'İndirilenler klasörüne aktar'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {done && <p className="ks-ok" role="status" translate="no">{done.path} · {done.files} dosya · {bytes(done.bytes)}</p>}
    </section>
  )
}

export function SystemPanel(_props: KorPanelProps) {
  const { data, error, isLoading, mutate } = useSWR('system:profile', () => systemApi.profile())
  const [changed, setChanged] = useState<string[] | null>(null)
  const [cleanOpen, setCleanOpen] = useState(0)
  const { busy, run } = useAction()
  useEffect(() => systemApi.onChanged((r) => { setChanged(r.changed); void mutate(r.profile, { revalidate: false }) }), [mutate])
  useEffect(() => {
    const on = () => setCleanOpen((n) => n + 1)
    window.addEventListener(CLEAN_EVENT, on)
    return () => window.removeEventListener(CLEAN_EVENT, on)
  }, [])
  const refresh = async () => {
    const r = await run(() => systemApi.refresh())
    if (r) {
      setChanged(r.changed)
      void mutate(r.profile, { revalidate: false })
    }
  }
  return (
    <div className="ks" data-kor-event="kor:system/changed kor:system/health">
      <div className="kg-card-head">
        <h2 className="ks-title">Sistem</h2>
        <button type="button" className="action-btn" onClick={() => void refresh()} disabled={busy} aria-label="Sistem profilini yeniden çıkar" title="Yenile" data-kor-cmd="system.refresh">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {changed && changed.length > 0 && <p className="ks-note" role="status" translate="no">Değişenler: {changed.join(', ')}</p>}
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading || !data ? (
        <p className="muted" role="status" data-kor-cmd="system.profile">Sistem profili çıkarılıyor…</p>
      ) : (
        <ProfileCard p={data} />
      )}
      <HealthCard onClean={() => setCleanOpen((n) => n + 1)} />
      <CleanCard open={cleanOpen} />
      <ComputerCard />
      <LayoutCard />
      <ExportCard />
    </div>
  )
}
