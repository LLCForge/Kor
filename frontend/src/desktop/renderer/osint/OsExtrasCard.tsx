// §27.10 işletim sistemine özgü entegrasyonlar (ADR 0052): yalnızca bu işletim sisteminin öğeleri, kur/kaldır
// (onaylı), Linux'ta portal izinleri tek ekrandan. Arka uç: ns/osextra.rs → kor_platform::os_integration.
import { Blocks } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { OsExtraStatus, PortalOutcome } from '../../ipc-osextra.ts'
import { errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { osextraApi } from './osextra-bridge.ts'
import { LABELS, VIA, reasonText, rowAction } from './osextra-model.ts'

const KEY = 'osextra:list'

const PORTAL_NAMES: Readonly<Record<PortalOutcome['portal'], string>> = {
  screencast: 'Ekran kaydı',
  'remote-desktop': 'Klavye ve fare',
  'file-chooser': 'Dosya erişimi',
  notification: 'Bildirimler',
}

function stateText(s: OsExtraStatus): string {
  if (s.kind === 'builtin') return s.installed ? 'Yerleşik' : 'Kapalı'
  if (s.kind === 'delegated') return VIA[s.via ?? ''] ?? 'Başka ayardan'
  if (s.installed) return 'Kurulu'
  if (s.available) return 'Kurulu değil'
  return s.reason === 'no-bundle' ? 'Bu kurulumda yok' : 'Kullanılamıyor'
}

export function OsExtrasCard() {
  const id = useId()
  const { data, error, mutate } = useSWR(KEY, () => osextraApi.list())
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [handoffUrl, setHandoffUrl] = useState('')
  const [portals, setPortals] = useState<PortalOutcome[] | null>(null)

  const run = async (s: OsExtraStatus) => {
    const action = rowAction(s)
    if (!action) return
    setActionError(null)
    setNote(null)
    if (action === 'portals') {
      const ok = await confirmAction({
        title: 'Portal izinleri',
        message:
          'Kor sırayla dört sistem penceresi açacak: ekran kaydı (ekran görüntüsü ve bilgisayar kullanımı), klavye ve fare (bilgisayar kullanımı), bir klasöre erişim ve bildirimler. Her birini ayrı ayrı onaylayabilir ya da reddedebilirsiniz; ekran ve girdi izinleri hatırlanır.',
        confirmLabel: 'İzinleri iste',
      })
      if (!ok) return
    } else {
      const ok = await confirmAction({
        title: action === 'install' ? 'Entegrasyon kurulsun mu?' : 'Entegrasyon kaldırılsın mı?',
        message:
          action === 'install'
            ? 'Kor yalnızca kendi dosyalarını yazar; kaldırınca hepsini siler.'
            : 'Kor’un bu entegrasyon için yazdığı her şey silinir; başka dosyalara dokunulmaz.',
        confirmLabel: action === 'install' ? 'Kur' : 'Kaldır',
        danger: action === 'uninstall',
      })
      if (!ok) return
    }
    setBusy(s.id)
    try {
      if (action === 'portals') setPortals(await osextraApi.portals())
      else if (action === 'install') await osextraApi.install(s.id, s.id === 'handoff' ? handoffUrl.trim() || null : null)
      else {
        const done = await osextraApi.uninstall(s.id)
        if (done.left) setNote(reasonText(done.left))
      }
      if (action === 'install' && s.id === 'powertoys-run') setNote('Eklentinin görünmesi için PowerToys’u yeniden başlatın.')
      if (action === 'install' && s.id === 'gnome-shell') setNote('Uzantı bir sonraki oturum açılışında üst çubukta görünür.')
      if (action === 'install' && s.id === 'shortcuts') setNote('Kestirmeler uygulamasında her kestirme için “Kestirme Ekle”yi onaylayın.')
      if (s.id === 'spotlight') await osextraApi.spotlightSync()
      await mutate()
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-cmd="osextra.list">
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <Blocks size={16} aria-hidden="true" />
          İşletim sistemi entegrasyonları
        </h3>
      </div>
      <p className="muted ks-note">Kor ilk açılışta kullanılabilenleri bir kez kurar. İstemediğinizi Kaldır ile kapatın: kaldırınca iz bırakmaz ve bir daha kendiliğinden kurulmaz. Aynısı terminalde: kor os-integration.</p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">Okunuyor…</p>
      ) : data.items.length === 0 ? (
        <p className="muted">Bu işletim sistemi için entegrasyon yok.</p>
      ) : (
        <ul className="ks-list" aria-label="Entegrasyonlar">
          {data.items.map((s) => {
            const action = rowAction(s)
            const meta = LABELS[s.id]
            const reason = s.kind === 'install' || s.kind === 'desktop' ? reasonText(s.reason) : null
            return (
              <li key={s.id} className="ks-row">
                <div>
                  <strong>{meta?.label ?? s.id}</strong>
                  <p className="muted ks-note">{meta?.desc}</p>
                  {reason && <p className="muted ks-note">{reason}</p>}
                  {s.id === 'wsl' && data.wsl.length > 0 && (
                    <p className="muted ks-note">
                      Dağıtımlar: <code translate="no">{data.wsl.join(', ')}</code>
                    </p>
                  )}
                  {s.id === 'handoff' && action === 'install' && (
                    <label className="ks-note">
                      Başka aygıtta açılacak https adresi (isteğe bağlı){' '}
                      <input
                        className="text-input"
                        type="url"
                        value={handoffUrl}
                        maxLength={2048}
                        placeholder="https://"
                        spellCheck={false}
                        translate="no"
                        onChange={(e) => setHandoffUrl(e.currentTarget.value)}
                      />
                    </label>
                  )}
                </div>
                <span className={`ks-chip${s.installed ? '' : ' muted'}`}>{stateText(s)}</span>
                {action && (
                  <button
                    type="button"
                    className={`btn small${action === 'uninstall' ? '' : ' primary'}`}
                    disabled={busy !== null}
                    aria-busy={busy === s.id}
                    onClick={() => void run(s)}
                    data-kor-cmd={action === 'portals' ? 'osextra.portals' : `osextra.${action}`}
                  >
                    {action === 'uninstall' ? 'Kaldır' : action === 'portals' ? 'İzinleri iste' : 'Kur'}
                  </button>
                )}
              </li>
            )
          })}
        </ul>
      )}
      {portals && (
        <ul className="ks-list" aria-label="Portal izinleri" data-kor-cmd="osextra.portals">
          {portals.map((p) => (
            <li key={p.portal} className="ks-row">
              <span>{PORTAL_NAMES[p.portal]}</span>
              <span className={`ks-chip${p.granted ? '' : ' muted'}`}>{p.granted ? 'Verildi' : 'Verilmedi'}</span>
            </li>
          ))}
        </ul>
      )}
      {note && <p className="muted ks-note" role="status">{note}</p>}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}
