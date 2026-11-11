// Ayarlar > Otomasyonlar / Okunabilirlik / Tuş düzeni (ADR 0050). Değerler kullanıcı config.toml'una yazılır;
// CLI ve TUI aynı dosyayı okur (`kor config explain auto|ui|keys`). Her değişiklik denetim günlüğüne düşer.
import { Wrench } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { AskThreshold, DoctorFix, UiPrefs } from '../../ipc-prefs.ts'
import { errorMessage } from '../api.ts'
import { prefsApi } from './api.ts'
import { IpcTraceRow } from './IpcTraceRow.tsx'
import './prefs.css'

/** Her otomatik davranışın Türkçe açıklaması (anahtar `auto.<key>`). */
const AUTO_LABELS: Record<string, string> = {
  compact: 'Bağlam dolunca konuşmayı otomatik özetle',
  memory_inject: 'Görev başında bilgi tabanı ve otomatik belleği isteme ekle',
  memory_record: 'Görev sonunda öğrenilenleri ve bellek notlarını kaydet',
  graph_watch: 'Oturum açıkken proje grafını dosya izleyiciyle güncel tut',
  hotspots: 'Graftan proje belleğine hotspots.md yaz',
  update: 'Doğrulanmış yeni sürümü kendiliğinden kur',
  catalog_refresh: 'Eskiyen model kataloğunu ve model listelerini tazele',
  routing: 'Görevleri kategorisine göre modele yönlendir (maliyet yönlendiricisi)',
  verify: 'Düzenlemeden sonra projenin denetleyicisini çalıştır',
  trigger_approve: 'Tetikleyicilerde (approval = auto) yıkıcı olmayan adımları kendiliğinden onayla',
  sound: 'Bildirim sesini çal',
  plan_intent: '"Önce araştır / dokunma" denince plan moduna geç',
  session_suggest: 'Oturum bitince başlık ve etiket öner',
}

function Row({ title, desc, children }: { title: string; desc?: ReactNode; children: ReactNode }) {
  return (
    <div className="g7-setting">
      <div className="g7-setting-text">
        <span className="g7-setting-title">{title}</span>
        {desc && <span className="g7-note">{desc}</span>}
      </div>
      <div className="g7-setting-control">{children}</div>
    </div>
  )
}

export function AutomationsSection() {
  const auto = useSWR('prefs:auto', () => prefsApi.auto())
  const agent = useSWR('prefs:agent', () => prefsApi.agent())
  const [error, setError] = useState<string | null>(null)
  const [fix, setFix] = useState<DoctorFix | null>(null)
  const [busy, setBusy] = useState(false)
  const run = async (p: Promise<unknown>, refresh: () => Promise<unknown>) => {
    setError(null)
    try {
      await p
      await refresh()
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const doctor = async () => {
    setBusy(true)
    setError(null)
    try {
      setFix(await prefsApi.doctorFix())
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="g7-stack" data-kor-cmd="prefs.auto">
      <p className="g7-note">Kor'un kendi kendine yaptığı hiçbir şey zorunlu değildir. Kapattığın davranış CLI, TUI ve masaüstünde birlikte kapanır; değişiklik denetim günlüğüne yazılır. CLI: <code>kor config explain auto</code></p>
      {auto.error && <p className="error-text" role="alert">{errorMessage(auto.error)}</p>}
      {!auto.data && !auto.error && <p className="muted">Yükleniyor…</p>}
      <div className="kor-prefs-list">
        {auto.data?.map((r) => (
          <Row
            key={r.key}
            title={AUTO_LABELS[r.key] ?? r.key}
            desc={
              <>
                <code translate="no">auto.{r.key}</code>
                {!r.fromDefault && <span className="kor-prefs-source"> · {r.source}</span>}
                {r.related.length > 0 && <span className="kor-prefs-related"> · ayrıca: {r.related.map(([k, v]) => `${k} = ${v}`).join(', ')}</span>}
              </>
            }
          >
            <input type="checkbox" checked={r.value} aria-label={AUTO_LABELS[r.key] ?? r.key} data-kor-cmd="prefs.setAuto" onChange={(e) => void run(prefsApi.setAuto(r.key, e.target.checked), () => auto.mutate())} />
          </Row>
        ))}
      </div>
      {agent.data && (
        <>
          <Row title="Açıklayıcı soru eşiği" desc="Ajan belirsizlikte ne zaman sorsun (agent.ask_threshold)">
            <select className="g7-select" value={agent.data.askThreshold} data-kor-cmd="prefs.setAgent" aria-label="Açıklayıcı soru eşiği" onChange={(e) => void run(prefsApi.setAgent('askThreshold', e.target.value as AskThreshold), () => agent.mutate())}>
              <option value="never">Asla sorma (en iyi tahmin, varsayımları listele)</option>
              <option value="critical">Kritikte sor (geri alınamaz, pahalı, çok yollu)</option>
              <option value="often">Sık sor</option>
            </select>
          </Row>
          <Row title="Özetlemeden önce sor" desc="Otomatik özetlemede neyin özetleneceğini göster, korunacakları seçtir (context.compact_confirm)">
            <input type="checkbox" checked={agent.data.compactConfirm} data-kor-cmd="prefs.setAgent" aria-label="Özetlemeden önce sor" onChange={(e) => void run(prefsApi.setAgent('compactConfirm', e.target.checked), () => agent.mutate())} />
          </Row>
          <Row title="Dur ve araştır kilidi" desc="Plan onaylanmadan ajan hiçbir şey yazamaz (plan.lock)">
            <input type="checkbox" checked={agent.data.planLock} data-kor-cmd="prefs.setAgent" aria-label="Dur ve araştır kilidi" onChange={(e) => void run(prefsApi.setAgent('planLock', e.target.checked), () => agent.mutate())} />
          </Row>
        </>
      )}
      <Row title="Sorunları gider" desc="kor doctor --fix: PATH, kabuk tamamlamaları, eksik klasörler, sahipsiz kilitler. Önce onay sorulur.">
        <button type="button" className="btn" onClick={() => void doctor()} disabled={busy} data-kor-cmd="prefs.doctorFix">
          <Wrench size={14} aria-hidden="true" /> Gider
        </button>
      </Row>
      <IpcTraceRow />
      {fix && (
        <div className="g7-stack">
          <p className="g7-note">{fix.fixed.length} düzeltme yapıldı{fix.failed.length > 0 ? `, ${fix.failed.length} başarısız` : ''}.</p>
          {fix.failed.length > 0 && <ul className="kor-prefs-warn">{fix.failed.map((f) => <li key={f} translate="no">{f}</li>)}</ul>}
          {fix.output && <pre className="kor-prefs-pre" translate="no">{fix.output}</pre>}
        </div>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

export function ReadabilitySection() {
  const { mutate: globalMutate } = useSWRConfig()
  const ui = useSWR('prefs:ui', () => prefsApi.ui())
  const [error, setError] = useState<string | null>(null)
  const set = async <K extends keyof UiPrefs>(name: K, value: UiPrefs[K]) => {
    setError(null)
    try {
      const next = await prefsApi.setUi(name, value)
      await ui.mutate(next, { revalidate: false })
      await globalMutate('prefs:ui', next, { revalidate: false })
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const u = ui.data
  if (!u) return ui.error ? <p className="error-text" role="alert">{errorMessage(ui.error)}</p> : <p className="muted">Yükleniyor…</p>
  const check = (name: 'wrap' | 'screenReader' | 'highContrast' | 'reducedMotion', title: string, desc: string) => (
    <Row title={title} desc={desc}>
      <input type="checkbox" checked={u[name]} aria-label={title} data-kor-cmd="prefs.setUi" onChange={(e) => void set(name, e.target.checked)} />
    </Row>
  )
  return (
    <div className="g7-stack" data-kor-cmd="prefs.ui">
      <p className="g7-note">TUI aynı ayarları okur (<code>kor config set ui.wrap_width 100</code>).</p>
      {check('wrap', 'Satır kaydırma', 'Uzun satırlar ve kod blokları kaydırılır (ui.wrap)')}
      <Row title="Yazı boyutu" desc="Sohbet metni (ui.font_size)">
        <input type="number" className="text-input" min={12} max={24} value={u.fontSize} aria-label="Yazı boyutu" data-kor-cmd="prefs.setUi" onChange={(e) => void set('fontSize', Number(e.target.value))} />
      </Row>
      <Row title="Satır aralığı" desc="ui.line_height">
        <input type="number" className="text-input" min={1.1} max={2.2} step={0.1} value={u.lineHeight} aria-label="Satır aralığı" data-kor-cmd="prefs.setUi" onChange={(e) => void set('lineHeight', Number(e.target.value))} />
      </Row>
      <Row title="Satır genişliği" desc="ui.max_width">
        <select className="g7-select" value={u.maxWidth} aria-label="Satır genişliği" data-kor-cmd="prefs.setUi" onChange={(e) => void set('maxWidth', e.target.value as UiPrefs['maxWidth'])}>
          <option value="narrow">Dar</option>
          <option value="medium">Orta</option>
          <option value="wide">Geniş</option>
          <option value="full">Tam</option>
        </select>
      </Row>
      <Row title="Yoğunluk" desc="ui.density">
        <select className="g7-select" value={u.density} aria-label="Yoğunluk" data-kor-cmd="prefs.setUi" onChange={(e) => void set('density', e.target.value as UiPrefs['density'])}>
          <option value="comfortable">Rahat</option>
          <option value="compact">Sıkışık</option>
        </select>
      </Row>
      <Row title="Kod yazı tipi" desc="Boş: temanın eş aralıklı yazı tipi (ui.code_font)">
        <input type="text" className="text-input" defaultValue={u.codeFont} maxLength={60} aria-label="Kod yazı tipi" data-kor-cmd="prefs.setUi" onBlur={(e) => e.target.value !== u.codeFont && void set('codeFont', e.target.value.trim())} />
      </Row>
      <Row title="Diff görünümü" desc="ui.diff_view">
        <select className="g7-select" value={u.diffView} aria-label="Diff görünümü" data-kor-cmd="prefs.setUi" onChange={(e) => void set('diffView', e.target.value as UiPrefs['diffView'])}>
          <option value="inline">Satır içi</option>
          <option value="split">Yan yana</option>
        </select>
      </Row>
      <Row title="Araç çıktısı ayrıntısı" desc="Görev sırasında da değişir (ui.tool_output, TUI'de /verbosity)">
        <select className="g7-select" value={u.toolOutput} aria-label="Araç çıktısı ayrıntısı" data-kor-cmd="prefs.setUi" onChange={(e) => void set('toolOutput', e.target.value as UiPrefs['toolOutput'])}>
          <option value="quiet">Sessiz</option>
          <option value="normal">Normal</option>
          <option value="verbose">Ayrıntılı</option>
        </select>
      </Row>
      {check('screenReader', 'Ekran okuyucu modu', 'Animasyon ve dönen göstergeler yok (ui.screen_reader)')}
      {check('highContrast', 'Yüksek karşıtlık', 'ui.high_contrast')}
      {check('reducedMotion', 'Azaltılmış hareket', 'ui.reduced_motion')}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

export function KeymapSection() {
  const { mutate: globalMutate } = useSWRConfig()
  const ui = useSWR('prefs:ui', () => prefsApi.ui())
  const km = useSWR('prefs:keymap', () => prefsApi.keymap())
  const [error, setError] = useState<string | null>(null)
  const setMode = async (keys: UiPrefs['keys']) => {
    setError(null)
    try {
      const next = await prefsApi.setUi('keys', keys)
      await globalMutate('prefs:ui', next, { revalidate: false })
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const binds = (title: string, map: Record<string, string> | undefined) =>
    map && Object.keys(map).length > 0 && (
      <Row title={title}>
        <pre className="kor-prefs-pre" translate="no">{Object.entries(map).map(([k, a]) => `${k} = ${a}`).join('\n')}</pre>
      </Row>
    )
  return (
    <div className="g7-stack" data-kor-cmd="prefs.keymap">
      <Row title="Tuş düzeni" desc="TUI ve mesaj kutusu (keys.mode); keymap.toml'daki mode bunu geçersiz kılar">
        <select className="g7-select" value={ui.data?.keys ?? 'default'} aria-label="Tuş düzeni" data-kor-cmd="prefs.setUi" onChange={(e) => void setMode(e.target.value as UiPrefs['keys'])}>
          <option value="default">Varsayılan</option>
          <option value="vim">Vim (normal / ekleme / görsel, : komutları)</option>
          <option value="emacs">Emacs</option>
        </select>
      </Row>
      <Row title="keymap.toml dosyaları" desc="Kullanıcı (config.toml'un yanında) ve proje (.kor/keymap.toml); [tui] ve [composer] tabloları">
        <button type="button" className="btn" onClick={() => void km.mutate()} data-kor-cmd="prefs.keymap">Yeniden oku</button>
      </Row>
      {km.data && km.data.files.length === 0 && <p className="g7-note">Henüz keymap.toml yok; yerleşik kısayollar geçerli.</p>}
      {km.data?.files.map((f) => <p key={f} className="g7-note" translate="no">{f}</p>)}
      {binds('TUI bağlamaları', km.data?.tui)}
      {binds('Mesaj kutusu bağlamaları', km.data?.composer)}
      {km.data && km.data.warnings.length > 0 && (
        <ul className="kor-prefs-warn" aria-label="Çakışmalar ve hatalar">
          {km.data.warnings.map((w) => <li key={w} translate="no">{w}</li>)}
        </ul>
      )}
      {(error || km.error) && <p className="error-text" role="alert">{error ?? errorMessage(km.error)}</p>}
    </div>
  )
}
