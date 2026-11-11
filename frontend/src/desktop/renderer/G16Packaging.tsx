// [g8] Paketleme (ADR 0032): paketle gelen kor CLI'ını onayla PATH'e ekleme/kaldırma ve "güncelleme var" bildirimi.
// PATH bir sistem değişikliğidir: yalnızca kullanıcı onay penceresinde kabul edince çağrılır; geri alınabilir.
import { Download, Terminal, Trash2, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { CliStatus, G16PackagingApi } from '../ipc-g16.ts'
import { api, errorMessage } from './api.ts'
import { confirmAction } from './ConfirmDialog.tsx'
import { openG16Panel } from './g16-bus.ts'

const packaging = api as unknown as G16PackagingApi

function cliText(s: CliStatus): string {
  if (!s.available) return 'Bu kurulumda paketlenmiş kor CLI yok (geliştirme derlemesi). CLI ayrıca kurulabilir.'
  if (s.byPackage) return `kor sistem paketiyle kuruldu ve zaten PATH'te: ${s.location ?? ''}`
  if (s.installed) return `kor terminalden çalıştırılabilir (${s.location ?? ''}). Yeni açılan terminallerde geçerlidir.`
  return 'kor terminalden çalıştırılamıyor. Masaüstüyle aynı sürüm, aynı ayar ve verilerle PATH\'e eklenebilir.'
}

export function CliPathSection() {
  const cli = useSWR('g16:cli', () => packaging.g16CliStatus())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const act = async (install: boolean) => {
    const s = cli.data
    const ok = await confirmAction(
      install
        ? {
            title: "kor CLI'ı PATH'e ekle",
            message:
              s?.location && /[\\/]kor(\.exe)?$/i.test(s.location)
                ? `${s.location} bağlantısı oluşturulacak; gerekirse ~/.local/bin kabuk profilinize eklenecek. Bulunan kabuklara tamamlamalar da kurulur (zsh, PowerShell, elvish profiline işaretli tek satır). Kaldır ile geri alınır.`
                : 'Kor klasörü kullanıcı PATH değişkeninize eklenecek (yönetici izni gerekmez). Bulunan kabuklara tamamlamalar da kurulur (PowerShell profiline işaretli tek satır). Kaldır ile geri alınır.',
            confirmLabel: 'Ekle',
          }
        : {
            title: "kor CLI'ı PATH'ten kaldır",
            message: "Kor'un PATH'e yaptığı değişiklik geri alınacak; başka kor kurulumlarına dokunulmaz.",
            confirmLabel: 'Kaldır',
            danger: true,
          },
    )
    if (!ok) return
    setBusy(true)
    setError(null)
    try {
      await cli.mutate(install ? packaging.g16CliInstall() : packaging.g16CliRemove(), { revalidate: false })
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const s = cli.data
  return (
    <section className="g16-section" aria-label="Komut satırı (kor)" data-kor-cmd="g16CliStatus">
      <h3 className="g16-section-title">Komut satırı (kor)</h3>
      {cli.isLoading && <p className="muted">Denetleniyor...</p>}
      {cli.error && (
        <p className="error-text" role="alert">
          {errorMessage(cli.error)}
        </p>
      )}
      {s && (
        <>
          <p className={s.installed ? '' : 'muted'} translate="no">
            {cliText(s)}
          </p>
          {s.available && !s.byPackage && (
            <div className="g16-actions">
              {!s.installed && (
                <button type="button" className="btn small" data-kor-cmd="g16CliInstall" disabled={busy} onClick={() => void act(true)}>
                  <Terminal size={14} aria-hidden="true" /> PATH'e ekle
                </button>
              )}
              {s.removable && (
                <button type="button" className="btn small" data-kor-cmd="g16CliRemove" disabled={busy} onClick={() => void act(false)}>
                  <Trash2 size={14} aria-hidden="true" /> PATH'ten kaldır
                </button>
              )}
            </div>
          )}
        </>
      )}
      {error && (
        <p className="error-text" role="alert">
          {error}
        </p>
      )}
    </section>
  )
}

/** Açılışta ve günde bir kez yapılan denetim yeni sürüm bulursa küçük bir bildirim (Rust: kor:g16/updateAvailable). */
export function UpdateNotice() {
  const [found, setFound] = useState<{ version: string | null; signed: boolean } | null>(null)
  useEffect(() => packaging.g16.onUpdateAvailable(setFound), [])
  if (!found) return null
  return (
    <div className="g16-notice" role="status" aria-live="polite" data-kor-event="kor:g16/updateAvailable">
      <span className="g16-notice-text">
        Kor <span translate="no">{found.version ?? ''}</span> hazır.
        {found.signed ? ' İmzası doğrulanarak indirilir; kurulum onayınızla yapılır.' : ' Sürüm sayfasından indirilebilir.'}
      </span>
      <button
        type="button"
        className="btn small primary"
        onClick={() => {
          setFound(null)
          openG16Panel()
        }}
      >
        <Download size={14} aria-hidden="true" /> Ayrıntılar
      </button>
      <button type="button" className="action-btn" aria-label="Bildirimi kapat" title="Bildirimi kapat" onClick={() => setFound(null)}>
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  )
}
