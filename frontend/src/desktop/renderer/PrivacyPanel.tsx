import { FolderOpen, ShieldCheck } from 'lucide-react'
import { useId, useRef, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { PrivacyClearTarget, PrivacyFolder } from '../g7-types.ts'
import { api, errorMessage } from './api.ts'
import { uiLocale } from './ui-locale.ts'

const kb = (bytes: number) => `${(bytes / 1024).toLocaleString(uiLocale(), { maximumFractionDigits: 1 })} KB`

interface PrivacyPanelProps {
  /** Sohbetler silinince arayüz yeni sohbete geçer */
  onSessionsCleared: () => void
}

export function PrivacyPanel({ onSessionsCleared }: PrivacyPanelProps) {
  const baseId = useId()
  const { mutate } = useSWRConfig()
  const { data, error, mutate: refresh } = useSWR('g7:privacy', () => api.g7GetPrivacyInfo())
  const autoMem = useSWR('g30:autoMemory', () => api.g30.getAutoMemory())
  const [pending, setPending] = useState<PrivacyClearTarget | null>(null)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const confirmRef = useRef<HTMLDialogElement>(null)

  const openFolder = (folder: PrivacyFolder) => {
    setActionError(null)
    api.g7OpenFolder(folder).catch((err: unknown) => setActionError(errorMessage(err)))
  }

  const askClear = (target: PrivacyClearTarget) => {
    setPending(target)
    setActionError(null)
    confirmRef.current?.showModal()
  }

  const clear = async (mode: 'trash' | 'delete') => {
    if (!pending) return
    setBusy(true)
    try {
      await api.g7ClearData(pending, mode)
      if (pending === 'sessions') {
        onSessionsCleared()
        await mutate('sessions')
      }
      await refresh()
      confirmRef.current?.close()
    } catch (err) {
      setActionError(errorMessage(err))
      confirmRef.current?.close()
    } finally {
      setBusy(false)
      setPending(null)
    }
  }

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>

  const rows: ReadonlyArray<{ label: string; path: string; detail: string }> = [
    { label: 'Sohbetler', path: data.sessionsDir, detail: `${data.sessionCount} sohbet; mesajlar, araç çağrıları ve sonuçları dahil` },
    { label: 'Kalıcı hafıza', path: data.memoryFile, detail: data.memoryExists ? 'Ajanın tüm sohbetlerde hatırladığı notlar' : 'Henüz yok' },
    { label: 'Denetim günlüğü', path: data.auditDir, detail: `${data.auditFiles} gün, ${kb(data.auditBytes)}; araç kararları (uzun alanlar kısaltılmış)` },
    { label: 'Ayarlar', path: data.settingsFile, detail: 'Model, izin kuralları, takma adlar, tema' },
    { label: 'Klasör güveni', path: data.trustFile, detail: 'Güvendiğin / güvenmediğin klasörler' },
    { label: 'Sistem istemi', path: data.promptFile, detail: 'Özel sistem istemin (varsa)' },
    { label: 'Model kataloğu', path: data.aaCacheFile, detail: 'Herkese açık model verisi ve Artificial Analysis puanları; günde bir yenilenir' },
    {
      label: 'API anahtarları',
      path: data.userDataDir,
      detail: data.keyEncryption ? 'API anahtarları işletim sisteminin şifreli deposuyla (safeStorage) şifreli' : 'Şifreleme yok: anahtarlar yalnızca bu oturumda bellekte',
    },
  ]

  return (
    <div className="g7-stack">
      <p className="g7-callout">
        <ShieldCheck size={16} aria-hidden="true" />
        <span>
          <strong>Telemetri yok.</strong> Uygulama kullanım verisi, analitik veya hata raporu toplamaz ve hiçbir yere göndermez. Her şey bu bilgisayarda
          saklanır.
        </span>
      </p>

      <section aria-labelledby={`${baseId}-automem`}>
        <h4 id={`${baseId}-automem`} className="g7-h4">Otomatik hafıza</h4>
        <div className="g7-setting">
          <div className="g7-setting-text">
            <span className="g7-setting-title">Sohbetlerden otomatik hafıza çıkar</span>
            <span className="g7-note">
              Her turdan sonra ucuz bir modelle kalıcı tercih/gerçekler çıkarılıp sana önerilir; yalnızca onayınla kalıcı hafızaya eklenir (hiçbir şey
              otomatik yazılmaz). Hassas konular ve sırlar dışlanır. Kapalıyken çıkarım yapılmaz (ek token harcanmaz).
            </span>
          </div>
          <div className="g7-setting-control">
            <input
              type="checkbox"
              checked={autoMem.data ?? true}
              onChange={(e) => void api.g30.setAutoMemory(e.target.checked).then(() => autoMem.mutate())}
              aria-label="Otomatik hafıza çıkarımı"
            />
          </div>
        </div>
      </section>

      <section aria-labelledby={`${baseId}-sent`}>
        <h4 id={`${baseId}-sent`} className="g7-h4">Ağa ne gidiyor?</h4>
        <ul className="g7-bullets">
          <li>
            <strong>Sohbet API'si:</strong> sohbet mesajların, sistem istemi (hafıza ve güvendiğin klasörde AGENTS.md dahil), araç sonuçları, ajanın
            okuduğu dosya içerikleri ve eklediğin dosyalar. Modelin gizlilik türü (Özel / Anonimleştirilmiş) model seçicide yazar.
          </li>
          <li>
            <strong>Model kataloğu:</strong> GitHub'dan herkese açık catalog.json isteği (GET, anahtarsız). Artificial Analysis'e uygulamadan istek gitmez; sohbet veya dosya verisi gönderilmez.
          </li>
          <li>
            <strong>Bağlantı denetimi:</strong> çevrimdışı algılama için sohbet sunucusuna içeriksiz HEAD isteği.
          </li>
        </ul>
      </section>

      <section aria-labelledby={`${baseId}-stored`}>
        <h4 id={`${baseId}-stored`} className="g7-h4">Nerede ne saklanıyor?</h4>
        <dl className="g7-dl g7-privacy-list">
          {rows.map((r) => (
            <div key={r.label}>
              <dt>{r.label}</dt>
              <dd>
                <span className="g7-mono g7-path">{r.path}</span>
                <span className="g7-note">{r.detail}</span>
              </dd>
            </div>
          ))}
        </dl>
        <div className="g7-button-row">
          <button type="button" className="btn small" onClick={() => openFolder('config')}>
            <FolderOpen size={14} aria-hidden="true" /> Ayar klasörü
          </button>
          <button type="button" className="btn small" onClick={() => openFolder('sessions')}>
            <FolderOpen size={14} aria-hidden="true" /> Sohbetler
          </button>
          <button type="button" className="btn small" onClick={() => openFolder('audit')}>
            <FolderOpen size={14} aria-hidden="true" /> Denetim günlüğü
          </button>
          <button type="button" className="btn small" onClick={() => openFolder('userData')}>
            <FolderOpen size={14} aria-hidden="true" /> Uygulama verisi
          </button>
        </div>
      </section>

      <section aria-labelledby={`${baseId}-clear`}>
        <h4 id={`${baseId}-clear`} className="g7-h4">Temizle</h4>
        <div className="g7-button-row">
          <button type="button" className="btn small" onClick={() => askClear('sessions')} disabled={data.sessionCount === 0}>
            Tüm sohbetleri sil ({data.sessionCount})
          </button>
          <button type="button" className="btn small" onClick={() => askClear('audit')} disabled={data.auditFiles === 0}>
            Denetim günlüğünü sil ({data.auditFiles})
          </button>
        </div>
        <p className="g7-note">Hafızayı Genel bölümündeki Hafıza düzenleyicisinden temizleyebilirsin.</p>
        {actionError && <p className="error-text" role="alert">{actionError}</p>}
      </section>

      <dialog ref={confirmRef} className="dialog g7-confirm" aria-labelledby={`${baseId}-confirm`} onClose={() => setPending(null)}>
        <div className="dialog-body">
          <h2 id={`${baseId}-confirm`} className="dialog-title">{pending === 'sessions' ? 'Tüm sohbetler silinsin mi?' : 'Denetim günlüğü silinsin mi?'}</h2>
          <p className="dialog-desc text-pretty">
            {pending === 'sessions'
              ? 'Kayıtlı tüm sohbetler ve açık (çalışmayan) sohbetler kapatılır. '
              : 'Araç kararlarının tüm kaydı silinir. '}
            Çöp kutusuna taşırsan geri alabilirsin. <strong>Kalıcı silme geri alınamaz.</strong>
          </p>
          <div className="dialog-actions">
            <button type="button" className="btn" onClick={() => confirmRef.current?.close()} disabled={busy}>
              Vazgeç
            </button>
            <div className="dialog-actions-right">
              <button type="button" className="btn" onClick={() => void clear('trash')} disabled={busy}>
                Çöp kutusuna taşı
              </button>
              <button type="button" className="btn g7-danger" onClick={() => void clear('delete')} disabled={busy}>
                Kalıcı olarak sil
              </button>
            </div>
          </div>
        </div>
      </dialog>
    </div>
  )
}
