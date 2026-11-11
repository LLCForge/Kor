// Yardımcı bileşenler (§24.2): liste, boyut, SHA-256 doğrulaması, kur/kaldır; ilerleme `kor:runtimeProgress`.
import { Download, RefreshCw, ShieldCheck, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { ComponentView, RuntimeTick } from '../../../ipc-providers.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { kor } from './bridge.ts'
import { formatBytes } from './catalog-view.ts'

export function ComponentsTab() {
  const { data, error, isLoading, mutate } = useSWR('components.list', () => kor.components.list(), { revalidateOnFocus: false })
  const [ticks, setTicks] = useState<Record<string, RuntimeTick>>({})
  const [busy, setBusy] = useState<string | null>(null)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  useEffect(() => kor.onRuntimeProgress((t) => setTicks((prev) => ({ ...prev, [t.id]: t }))), [])

  const act = async (c: ComponentView, fn: () => Promise<ComponentView[]>, ok: string) => {
    setBusy(c.name)
    setMsg(null)
    try {
      await mutate(await fn(), { revalidate: false })
      setMsg({ ok: true, text: ok })
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    } finally {
      setBusy(null)
    }
  }

  // İndirme onayı (ad + boyut) Rust tarafındaki yerel pencerede bir kez sorulur.
  const install = (c: ComponentView) => act(c, () => kor.components.add(c.name), `${c.name} kuruldu ve doğrulandı.`)

  const all = async (label: string, fn: () => Promise<ComponentView[]>, ok: string) => {
    setBusy(label)
    setMsg(null)
    try {
      await mutate(await fn(), { revalidate: false })
      setMsg({ ok: true, text: ok })
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    } finally {
      setBusy(null)
    }
  }

  // Kurulu dosyalar kurulumda kaydedilen SHA-256 özetleriyle karşılaştırılır; bozuk kopya silinir.
  const verify = async () => {
    setBusy('verify')
    setMsg(null)
    try {
      const out = await kor.components.verify()
      const bad = out.filter((v) => v.state === 'broken')
      setMsg(
        out.length === 0
          ? { ok: true, text: 'Kurulu bileşen yok.' }
          : bad.length === 0
            ? { ok: true, text: `${out.length} bileşen sağlam.` }
            : { ok: false, text: `Bozuk: ${bad.map((v) => `${v.name} (${v.detail})`).join(', ')}. Silindi; ilk kullanımda yeniden indirilecek.` },
      )
      await mutate()
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    } finally {
      setBusy(null)
    }
  }

  const remove = async (c: ComponentView) => {
    const ok = await confirmAction({
      title: `${c.name} kaldırılsın mı?`,
      message: `${c.about} bu bilgisayardan silinir. Gerektiğinde yeniden indirilir (${formatBytes(c.size)}).`,
      confirmLabel: 'Kaldır',
      danger: true,
    })
    if (ok) await act(c, () => kor.components.remove(c.name), `${c.name} kaldırıldı.`)
  }

  return (
    <section className="kp-section" aria-labelledby="kp-comp-h" data-kor-event="kor:runtimeProgress">
      <div className="kp-head">
        <h3 id="kp-comp-h" className="kp-h">
          Yardımcı bileşenler
        </h3>
        <span className="kp-grow" />
        <button type="button" className="btn small" data-kor-cmd="components.verify" onClick={() => void verify()} disabled={busy !== null}>
          <ShieldCheck size={14} aria-hidden="true" /> {busy === 'verify' ? 'Doğrulanıyor...' : 'Doğrula'}
        </button>
        <button type="button" className="btn small" data-kor-cmd="components.update" onClick={() => void all('update', () => kor.components.update(), 'Bileşenler güncel.')} disabled={busy !== null}>
          <RefreshCw size={14} aria-hidden="true" /> Güncelle
        </button>
        <button type="button" className="btn small" data-kor-cmd="components.installAll" onClick={() => void all('all', () => kor.components.installAll(), 'Bütün bileşenler kuruldu ve doğrulandı.')} disabled={busy !== null}>
          <Download size={14} aria-hidden="true" /> {busy === 'all' ? 'Kuruluyor...' : 'Hepsini kur'}
        </button>
      </div>
      <p className="g7-note text-pretty">
        Bazı araçlar (PDF, OCR, tarayıcı, Python) gerektiğinde Kor'un kendi klasörüne indirilen bileşenleri kullanır. Her indirme sabitlenmiş SHA-256 özetiyle
        doğrulanmadan kurulmaz; gizlilik modunda indirme yapılmaz.
      </p>
      {error && (
        <p className="error-text" role="alert">
          {errorMessage(error)}
        </p>
      )}
      {msg && (
        <p className={msg.ok ? 'g7-note' : 'error-text'} role="status">
          {msg.text}
        </p>
      )}
      {isLoading && <p className="muted">Yükleniyor...</p>}
      {data && data.length === 0 && <p className="kp-empty">Bileşen bildirimi boş.</p>}
      {data && data.length > 0 && (
        <ul className="kp-list" data-kor-cmd="components.list">
          {data.map((c) => {
            const t = ticks[c.name]
            const running = busy === c.name
            return (
              <li key={c.name} className="kp-row">
                <div className="kp-row-main">
                  <span className="kp-name" translate="no">
                    {c.name}
                  </span>
                  <span className="g7-mono">{c.version}</span>
                  {c.installed ? (
                    <span className="badge g9-on" title={c.sha256 ? `SHA-256 ${c.sha256}` : undefined}>
                      <ShieldCheck size={12} aria-hidden="true" /> Kurulu, doğrulandı
                    </span>
                  ) : (
                    <span className="badge neutral">{c.size === null ? 'Bu sistem için derleme yok' : 'Kurulu değil'}</span>
                  )}
                  <span className="kp-grow" />
                  {c.installed ? (
                    <button type="button" className="action-btn" data-kor-cmd="components.remove" onClick={() => void remove(c)} disabled={busy !== null} aria-label={`${c.name} bileşenini kaldır`} title="Kaldır">
                      <Trash2 size={14} aria-hidden="true" />
                    </button>
                  ) : (
                    c.size !== null && (
                      <button type="button" className="btn small" data-kor-cmd="components.add" onClick={() => void install(c)} disabled={busy !== null}>
                        <Download size={14} aria-hidden="true" /> {running ? 'Kuruluyor...' : `Kur (${formatBytes(c.size)})`}
                      </button>
                    )
                  )}
                </div>
                <p className="kp-small">{c.about}</p>
                {c.path && (
                  <p className="kp-small g7-mono" translate="no">
                    {c.path}
                  </p>
                )}
                {t && t.phase !== 'bitti' && (running || t.phase === 'hata') && (
                  <div className="kp-progress" role="status">
                    {t.percent !== null && <progress max={100} value={t.percent} aria-label={`${c.name} indiriliyor`} />}
                    <span className={t.phase === 'hata' ? 'error-text kp-small' : 'kp-small'}>{t.line}</span>
                  </div>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
