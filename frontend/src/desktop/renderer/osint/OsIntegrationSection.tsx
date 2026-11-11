// Ayarlar > İşletim sistemi (§25, §27.5, §27.10; ADR 0052): "Kor ile Aç" sağ tık menüsü (ilk açılışta bir kez kurulur; kur/kaldır onayla),
// genişletilmiş alt menü, global kısayol, son projeler (atlama listesi / Dock menüsü); ~/Kor klasörü ve OS'a özgü
// entegrasyonlar kendi kartlarında. Rust: ns/osint.rs (+ ns/korfolder.rs, ns/osextra.rs).
import { Keyboard, MousePointerClick, RefreshCw, History } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { KorFolderCard } from './KorFolderCard.tsx'
import { OsExtrasCard } from './OsExtrasCard.tsx'
import { osintApi } from './useOsint.ts'
import '../panels/graph/panel.css'
import '../panels/security/panel.css'

function ContextMenuCard() {
  const id = useId()
  const { data, error, mutate } = useSWR('osint:menu', () => osintApi.menuStatus())
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const installed = data?.installed === true
  const extended = data?.extended === true

  const install = async (ext: boolean) => {
    const ok = await confirmAction({
      title: 'Sağ tık menüsü kurulsun mu?',
      message:
        'Dosya yöneticisinde klasör ve dosyalara "Kor CLI ile aç", "Kor Desktop ile aç" ve "Kor ile düzenle" eklenir. Yalnızca kullanıcı hesabınıza yazılır (yönetici izni gerekmez) ve buradan tamamen kaldırılabilir.',
      confirmLabel: 'Kur',
    })
    if (!ok) return
    setBusy(true)
    setActionError(null)
    try {
      await mutate(await osintApi.menuInstall(ext), { revalidate: false })
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const uninstall = async () => {
    const ok = await confirmAction({
      title: 'Sağ tık menüsü kaldırılsın mı?',
      message: "Kor'un eklediği menü öğeleri ve kayıtlar silinir; başka hiçbir şeye dokunulmaz.",
      confirmLabel: 'Kaldır',
      danger: true,
    })
    if (!ok) return
    setBusy(true)
    setActionError(null)
    try {
      await mutate(await osintApi.menuUninstall(), { revalidate: false })
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <MousePointerClick size={16} aria-hidden="true" />
          Sağ tık menüsü
        </h3>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Menü durumunu yenile" title="Yenile" data-kor-cmd="osint.menuStatus">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      <p className="muted ks-note">
        Dosya yöneticisinde (Explorer, Finder, Nautilus, Dolphin, Thunar, Nemo…) bir klasöre ya da dosyaya sağ tıklayınca Kor'u o projede açar. Birden çok dosya seçip "Kor ile düzenle" toplu işlem başlatır. Kor ilk açılışta menüyü bir kez kurar; kaldırırsanız bir daha kendiliğinden kurulmaz.
      </p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="osint.menuStatus">Durum okunuyor…</p>
      ) : (
        <div className="ks-row" data-kor-cmd="osint.menuStatus">
          <span className={`ks-chip${installed ? '' : ' muted'}`}>{installed ? 'Kurulu' : 'Kurulu değil'}</span>
          {installed ? (
            <button type="button" className="btn small danger" onClick={() => void uninstall()} disabled={busy} data-kor-cmd="osint.menuUninstall">
              Kaldır…
            </button>
          ) : (
            <button type="button" className="btn small primary" onClick={() => void install(extended)} disabled={busy} data-kor-cmd="osint.menuInstall">
              Kur…
            </button>
          )}
        </div>
      )}
      <label className="ks-row">
        <input
          type="checkbox"
          checked={extended}
          disabled={busy || !data || !installed}
          onChange={(e) => void install(e.currentTarget.checked)}
          data-kor-cmd="osint.menuInstall"
        />
        <span>Alt menü: "Projeyi açıkla", "Hata bul", "Bu dosyayı test et" görevi doğrudan başlatır</span>
      </label>
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}

function HotkeyCard() {
  const id = useId()
  const { data, error, mutate } = useSWR('osint:hotkey', () => osintApi.hotkeyGet())
  const [value, setValue] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const shown = value ?? data?.hotkey ?? ''

  const save = async (next: string) => {
    setBusy(true)
    setActionError(null)
    try {
      await mutate(await osintApi.hotkeySet(next), { revalidate: false })
      setValue(null)
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <Keyboard size={16} aria-hidden="true" />
          Global kısayol
        </h3>
      </div>
      <p className="muted ks-note">Hangi uygulamada olursanız olun Kor'u öne getirir ve komut paletini açar. Boş bırakmak kısayolu kapatır.</p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : (
        <form
          className="ks-row"
          onSubmit={(e: FormEvent) => {
            e.preventDefault()
            void save(shown)
          }}
          data-kor-cmd="osint.hotkeyGet"
        >
          <label htmlFor={`${id}-k`} className="sr-only">Kısayol</label>
          <input id={`${id}-k`} className="text-input" value={shown} onChange={(e) => setValue(e.currentTarget.value)} placeholder={data?.default ?? 'CmdOrCtrl+Shift+K'} maxLength={64} spellCheck={false} translate="no" disabled={busy || !data} />
          <button type="submit" className="btn small primary" disabled={busy || !data} data-kor-cmd="osint.hotkeySet">Kaydet</button>
          <button type="button" className="btn small" disabled={busy || !data} onClick={() => void save(data?.default ?? '')} data-kor-cmd="osint.hotkeySet">Varsayılan</button>
          <button type="button" className="btn small" disabled={busy || !data} onClick={() => void save('')} data-kor-cmd="osint.hotkeySet">Kapat</button>
          {data && <span className={`ks-chip${data.active ? '' : ' muted'}`}>{data.active ? 'Etkin' : 'Kapalı'}</span>}
        </form>
      )}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}

function RecentCard() {
  const id = useId()
  const { data, error } = useSWR('osint:recent', () => osintApi.recent())
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <History size={16} aria-hidden="true" />
          Görev çubuğu ve Dock
        </h3>
      </div>
      <p className="muted ks-note">
        Görev çubuğu atlama listesi (Windows), Dock menüsü (macOS) ve masaüstü eylemleri (Linux) son 5 projeyi, "Yeni görev" ve "Çalışan görevler"i gösterir. Bir klasörü Kor penceresine (macOS'ta Dock simgesine de) bırakmak onu proje olarak açar; Windows görev çubuğu simgesine bırakmayı desteklemez. kor://open, kor://task ve kor://resume bağlantıları buradan açılır; bağlantılar her zaman onay ister.
      </p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="osint.recent">Okunuyor…</p>
      ) : data.length === 0 ? (
        <p className="muted" data-kor-cmd="osint.recent">Henüz açılmış proje yok.</p>
      ) : (
        <ul className="ks-list" aria-label="Son projeler" data-kor-cmd="osint.recent">
          {data.slice(0, 5).map((p) => (
            <li key={p} className="ks-row">
              <code translate="no" title={p}>{p}</code>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

export function OsIntegrationSection() {
  return (
    <>
      <ContextMenuCard />
      <HotkeyCard />
      <RecentCard />
      <KorFolderCard />
      <OsExtrasCard />
    </>
  )
}
