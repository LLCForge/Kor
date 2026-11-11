// [g16] Dağıtım ve tanılama: sürüm/güncelleme (195), yapılandırma dosyası (199), yerel çökme raporları (200).
// Dosya seçimi, onay ve ağ istekleri ana süreçte yapılır; bu bileşen yalnızca durumu gösterir ve eylemi başlatır.
import { Download, FileDown, FileUp, FolderOpen, RefreshCw, RotateCw, Send } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import useSWR from 'swr'
import type { ConfigImportResult, Distribution, UpdateStatus } from '../ipc-g16.ts'
import { api, errorMessage } from './api.ts'
import { closeG16Panel, useG16PanelOpen } from './g16-bus.ts'
import { CliPathSection, UpdateNotice } from './G16Packaging.tsx'
import { G7Modal } from './G7Modal.tsx'
import './styles/g16.css'

const DISTRIBUTION_LABEL: Readonly<Record<Distribution, string>> = {
  nsis: 'Windows kurulumu',
  msi: 'Windows MSI (kurumsal)',
  portable: 'Taşınabilir sürüm',
  mac: 'macOS',
  appimage: 'Linux AppImage',
  deb: 'Linux .deb paketi',
  rpm: 'Linux .rpm paketi',
  dev: 'Geliştirme',
}

function phaseText(status: UpdateStatus): string {
  switch (status.phase) {
    case 'idle':
      return 'Henüz denetlenmedi.'
    case 'checking':
      return 'Denetleniyor...'
    case 'available':
      return `${status.available ?? 'Yeni sürüm'} bulundu.`
    case 'not-available':
      return 'Uygulama güncel.'
    case 'downloading':
      return `${status.available ?? 'Güncelleme'} indiriliyor (yalnızca değişen parçalar): %${status.percent ?? 0}`
    case 'downloaded':
      return `${status.available ?? 'Güncelleme'} indirildi; yeniden başlatınca kurulur.`
    case 'error':
      return status.error ?? 'Güncelleme alınamadı.'
  }
}

const formatDate = (ms: number) => new Date(ms).toLocaleString(uiLocale())
const formatSize = (bytes: number) => (bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`)

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="g16-section" aria-label={title}>
      <h3 className="g16-section-title">{title}</h3>
      {children}
    </section>
  )
}

function importSummary(result: ConfigImportResult): string {
  const parts = [
    result.applied.length > 0 ? `Uygulanan ayarlar: ${result.applied.join(', ')}.` : 'Ayar değişmedi.',
    result.systemPrompt ? 'Sistem istemi güncellendi.' : '',
    result.outputStyle ? 'Çıktı stili güncellendi.' : '',
    result.skipped.length > 0 ? `Güvenlik nedeniyle atlananlar (yalnızca settings.json'dan düzenlenir): ${result.skipped.join(', ')}.` : '',
    'Model ve izin ayarları yeni sohbetlerde etkin olur.',
  ]
  return parts.filter(Boolean).join(' ')
}

function G16Body() {
  const update = useSWR('g16:update', () => api.g16UpdateStatus(), {
    refreshInterval: (d) => (d && (d.phase === 'checking' || d.phase === 'downloading') ? 1000 : 0),
  })
  const crash = useSWR('g16:crash', () => api.g16CrashList())
  const [busy, setBusy] = useState<string | null>(null)
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null)

  const run = async (key: string, action: () => Promise<string | null>) => {
    setBusy(key)
    setMessage(null)
    try {
      const text = await action()
      if (text) setMessage({ text, error: false })
    } catch (err) {
      setMessage({ text: errorMessage(err), error: true })
    } finally {
      setBusy(null)
    }
  }

  const status = update.data
  const info = crash.data

  return (
    <div className="g16-body">
      <Section title="Sürüm ve güncellemeler">
        {update.error && <p className="error-text" role="alert">{errorMessage(update.error)}</p>}
        {status && (
          <>
            <p>
              Sürüm <strong>{status.version}</strong> · {DISTRIBUTION_LABEL[status.distribution]}
            </p>
            {status.enabled ? (
              <>
                <p className="muted">
                  Güncelleme sunucusu: {status.host} ({status.source === 'kurumsal' ? 'kurumsal yapılandırma' : 'settings.json'})
                </p>
                <p role="status">{phaseText(status)}</p>
                <div className="g16-actions">
                  <button
                    type="button"
                    className="btn small"
                    disabled={busy !== null || status.phase === 'checking' || status.phase === 'downloading'}
                    onClick={() =>
                      void run('check', async () => {
                        await update.mutate(api.g16UpdateCheck(), { revalidate: false })
                        return null
                      })
                    }
                  >
                    <RefreshCw size={14} aria-hidden="true" /> Güncellemeleri denetle
                  </button>
                  {status.phase === 'available' && (
                    <button
                      type="button"
                      className="btn small primary"
                      data-kor-cmd="g16UpdateInstall"
                      disabled={busy !== null}
                      onClick={() =>
                        void run('download', async () => {
                          await update.mutate(api.g16UpdateInstall() as unknown as Promise<UpdateStatus>, { revalidate: false })
                          return null
                        })
                      }
                    >
                      <Download size={14} aria-hidden="true" /> {status.signed ? 'İndir (imza doğrulanır)' : 'Sürüm sayfasını aç'}
                    </button>
                  )}
                  {status.phase === 'downloaded' && (
                    <button
                      type="button"
                      className="btn small primary"
                      disabled={busy !== null}
                      onClick={() =>
                        void run('install', async () => {
                          await api.g16UpdateInstall()
                          return null
                        })
                      }
                    >
                      <RotateCw size={14} aria-hidden="true" /> Yeniden başlat ve kur
                    </button>
                  )}
                </div>
              </>
            ) : (
              <p className="muted">Otomatik güncelleme kapalı: {status.reason}</p>
            )}
          </>
        )}
      </Section>

      <CliPathSection />

      <Section title="Yapılandırma dosyası">
        <p className="muted">
          Tüm ayarlar (settings.json, sistem istemi, çıktı stili) tek bir JSON dosyasına yazılır ve başka bir kurulumda aynen uygulanabilir. İzin modu, izin
          kuralları, kabuk, ek klasörler, güncelleme ve rapor adresleri içe aktarılmaz; yalnızca kendi settings.json dosyandan düzenlenir.
        </p>
        <div className="g16-actions">
          <button
            type="button"
            className="btn small"
            disabled={busy !== null}
            onClick={() => void run('export', async () => {
              const file = await api.g16ConfigExport()
              return file ? `Yapılandırma kaydedildi: ${file}` : null
            })}
          >
            <FileDown size={14} aria-hidden="true" /> Dışa aktar
          </button>
          <button
            type="button"
            className="btn small"
            disabled={busy !== null}
            onClick={() => void run('import', async () => {
              const result = await api.g16ConfigImport()
              return result ? importSummary(result) : null
            })}
          >
            <FileUp size={14} aria-hidden="true" /> İçe aktar
          </button>
        </div>
      </Section>

      <Section title="Çökme raporları">
        <p className="muted">
          Çökme dökümleri yalnızca bu bilgisayarda tutulur; hiçbir şey kendiliğinden gönderilmez. Dışa aktarılan rapor maskelidir (anahtarlar, ev klasörü,
          kullanıcı adı) ve döküm dosyalarının kendisini değil, yalnızca ad, boyut ve özetlerini içerir.
        </p>
        {crash.error && <p className="error-text" role="alert">{errorMessage(crash.error)}</p>}
        {info && info.events.length === 0 && info.dumps.length === 0 && <p>Kayıtlı çökme yok.</p>}
        {info && info.events.length > 0 && (
          <ul className="g16-list" aria-label="Çökme olayları">
            {info.events.map((e) => (
              <li key={`${e.at}-${e.process}`}>
                <span className="g16-mono">{formatDate(e.at)}</span> {e.process}: {e.reason}
                {e.exitCode !== null ? ` (çıkış kodu ${e.exitCode})` : ''}
              </li>
            ))}
          </ul>
        )}
        {info && info.dumps.length > 0 && (
          <ul className="g16-list" aria-label="Döküm dosyaları">
            {info.dumps.map((d) => (
              <li key={d.name}>
                <span className="g16-mono">{d.name}</span> · {formatSize(d.size)} · {formatDate(d.mtimeMs)}
              </li>
            ))}
          </ul>
        )}
        <div className="g16-actions">
          <button
            type="button"
            className="btn small"
            disabled={busy !== null}
            onClick={() => void run('crash-export', async () => {
              const file = await api.g16CrashExport()
              return file ? `Maskeli rapor kaydedildi: ${file}` : null
            })}
          >
            <Download size={14} aria-hidden="true" /> Maskeli raporu dışa aktar
          </button>
          {info?.uploadHost && (
            <button
              type="button"
              className="btn small"
              disabled={busy !== null}
              onClick={() => void run('crash-send', async () => ((await api.g16CrashSend()) ? `Rapor ${info.uploadHost} adresine gönderildi.` : null))}
            >
              <Send size={14} aria-hidden="true" /> {info.uploadHost} adresine gönder
            </button>
          )}
          <button
            type="button"
            className="btn small"
            disabled={busy !== null}
            onClick={() =>
              void run('crash-open', async () => {
                await api.g16CrashOpenFolder()
                return null
              })
            }
          >
            <FolderOpen size={14} aria-hidden="true" /> Döküm klasörünü aç
          </button>
        </div>
      </Section>

      <p className={message?.error ? 'error-text' : 'muted'} role={message?.error ? 'alert' : 'status'} aria-live="polite">
        {message?.text ?? ''}
      </p>
    </div>
  )
}

function G16Dialog() {
  const open = useG16PanelOpen()
  return (
    <G7Modal open={open} onClose={closeG16Panel} titleId="g16-title" title="Dağıtım ve tanılama" className="g16-panel">
      {open && <G16Body />}
    </G7Modal>
  )
}

/** Panel + açılışta/günlük denetimin "güncelleme var" bildirimi (panel kapalıyken de dinler). */
export function G16Panel() {
  return (
    <>
      <G16Dialog />
      <UpdateNotice />
    </>
  )
}

import { uiLocale } from './ui-locale.ts'