// Ayarlar > Kabuk (§15.4): kabuk entegrasyonu kur/kaldır (önce eklenecek blok gösterilir), son başarısız komut,
// `kor why` / `kor fix` karşılıkları. Önerilen düzeltme yalnızca onayla çalışır. Rust: ns/shell.rs.
import { Play, RefreshCw, Sparkles, TerminalSquare, Wrench } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { ShellChange } from '../../ipc-shell.ts'
import { errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { shellApi } from '../panels/security/bridge.ts'
import { shellStateText, splitShells } from './shell-model.ts'
import '../panels/graph/panel.css'
import '../panels/security/panel.css'

const SHELL_KEY = 'g5:shell'

function Integration() {
  const id = useId()
  const { data, error, mutate } = useSWR(SHELL_KEY, () => shellApi.status())
  const [preview, setPreview] = useState<{ change: ShellChange; install: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const show = async (shell: string, install: boolean) => {
    setBusy(true)
    setActionError(null)
    setMessage(null)
    try {
      const change = await (install ? shellApi.install(shell, false) : shellApi.uninstall(shell, false))
      if (change.changed) setPreview({ change, install })
      else {
        setPreview(null)
        setMessage(install ? `${shell} için entegrasyon zaten kurulu.` : `${shell} başlangıç dosyasında Kor bloğu yok.`)
      }
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const apply = async () => {
    if (!preview) return
    const { change, install } = preview
    const ok = await confirmAction({
      title: install ? `${change.shell} entegrasyonu kurulsun mu?` : `${change.shell} entegrasyonu kaldırılsın mı?`,
      message: install
        ? `${change.path} dosyası gösterilen blokla güncellenecek. Yeni açılan kabuklarda geçerli olur.`
        : `${change.path} dosyasındaki Kor bloğu çıkarılacak. Yeni açılan kabuklarda geçerli olur.`,
      confirmLabel: install ? 'Kur' : 'Kaldır',
      danger: !install,
    })
    if (!ok) return
    setBusy(true)
    setActionError(null)
    try {
      const done = await (install ? shellApi.install(change.shell, true) : shellApi.uninstall(change.shell, true))
      setPreview(null)
      setMessage(done.written ? `${done.path} güncellendi.` : 'Değişecek bir şey yoktu.')
      await mutate()
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const groups = splitShells(data?.shells ?? [])

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <TerminalSquare size={16} aria-hidden="true" />
          Kabuk entegrasyonu
        </h3>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Kabuk durumunu yenile" title="Yenile" data-kor-cmd="shell.status">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      <p className="muted ks-note">Başarısız komutun kendisi, çıkış kodu ve son çıktı satırları yalnızca bu bilgisayarda kaydedilir; hiçbir komut geçmişi onayınız olmadan dışarı gönderilmez.</p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="shell.status">Kabuklar okunuyor…</p>
      ) : (
        <>
          <ul className="ks-list" aria-label="Bu bilgisayardaki kabuklar" data-kor-cmd="shell.status">
            {groups.present.map((s) => (
              <li key={s.shell} className="ks-row">
                <code translate="no" title={s.path}>{s.shell}{s.isDefault ? ' (varsayılan)' : ''}</code>
                <span className={`ks-chip${s.installed ? '' : ' muted'}`}>{shellStateText(s)}</span>
                <button type="button" className="btn small" onClick={() => void show(s.shell, !s.installed)} disabled={busy} data-kor-cmd={s.installed ? 'shell.uninstall' : 'shell.install'}>
                  {s.installed ? 'Kaldır…' : 'Kur…'}
                </button>
              </li>
            ))}
          </ul>
          {groups.absent.length > 0 && (
            <details className="ks-more">
              <summary>Diğer kabuklar (bu bilgisayarda yok)</summary>
              <ul className="ks-list" aria-label="Bu bilgisayarda olmayan kabuklar">
                {groups.absent.map((s) => (
                  <li key={s.shell} className="ks-row">
                    <code translate="no">{s.shell}</code>
                    <span className="ks-chip muted">{shellStateText(s)}</span>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </>
      )}
      {preview && (
        <div className="ks-preview">
          <p>
            {preview.install ? 'Eklenecek blok' : 'Çıkarılacak blok'}: <code translate="no">{preview.change.path}</code>
          </p>
          <pre className="ks-code" translate="no">{preview.change.block}</pre>
          <div className="ks-actions">
            <button type="button" className={`btn small ${preview.install ? 'primary' : 'danger'}`} onClick={() => void apply()} disabled={busy} data-kor-cmd={preview.install ? 'shell.install' : 'shell.uninstall'}>
              {preview.install ? 'Kur' : 'Kaldır'}
            </button>
            <button type="button" className="btn small" onClick={() => setPreview(null)} disabled={busy}>Vazgeç</button>
          </div>
        </div>
      )}
      {message && <p className="muted" role="status">{message}</p>}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}

function LastFailure() {
  const id = useId()
  const { data } = useSWR(SHELL_KEY, () => shellApi.status())
  const [explanation, setExplanation] = useState<string | null>(null)
  const [command, setCommand] = useState<string | null>(null)
  const [result, setResult] = useState<{ code: number; output: string } | null>(null)
  const [busy, setBusy] = useState<'why' | 'fix' | 'run' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const last = data?.last ?? null

  const ask = async (kind: 'why' | 'fix') => {
    setBusy(kind)
    setError(null)
    setResult(null)
    try {
      if (kind === 'why') {
        setExplanation((await shellApi.why()).explanation)
        setCommand(null)
      } else {
        const r = await shellApi.fix()
        setExplanation(r.explanation)
        setCommand(r.command)
      }
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const run = async () => {
    if (!command?.trim()) return
    const ok = await confirmAction({ title: 'Önerilen komut çalıştırılsın mı?', message: `${command.trim()} — Komut, başarısız komutun klasöründe ve kabuğunda çalışır.`, confirmLabel: 'Çalıştır', danger: true })
    if (!ok) return
    setBusy('run')
    setError(null)
    try {
      setResult(await shellApi.runFix(command.trim()))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Wrench size={16} aria-hidden="true" />
        Son başarısız komut
      </h3>
      {!data ? (
        <p className="muted" role="status">Okunuyor…</p>
      ) : !last ? (
        <p className="muted ks-empty">Kayıtlı başarısız komut yok. Entegrasyonu kurduğunuz bir kabukta bir komut hata verdiğinde burada görünür.</p>
      ) : (
        <>
          <dl className="ks-facts">
            <dt>Komut</dt>
            <dd translate="no"><code>{last.command}</code></dd>
            <dt>Çıkış kodu</dt>
            <dd>{last.exit}</dd>
            <dt>Kabuk</dt>
            <dd translate="no">{last.shell}</dd>
            <dt>Klasör</dt>
            <dd translate="no">{last.cwd}</dd>
          </dl>
          {last.output.length > 0 && <pre className="ks-code" translate="no">{last.output.join('\n')}</pre>}
          <div className="ks-actions">
            <button type="button" className="btn small" onClick={() => void ask('why')} disabled={busy !== null} data-kor-cmd="shell.why">
              <Sparkles size={14} aria-hidden="true" />
              {busy === 'why' ? 'Açıklanıyor…' : 'Neden başarısız?'}
            </button>
            <button type="button" className="btn small" onClick={() => void ask('fix')} disabled={busy !== null} data-kor-cmd="shell.fix">
              <Wrench size={14} aria-hidden="true" />
              {busy === 'fix' ? 'Düzeltme aranıyor…' : 'Düzeltme öner'}
            </button>
          </div>
          <p className="muted ks-note">Açıklama için kayıt (gizli değerler maskelenerek) varsayılan modele gönderilir; gizlilik modunda yalnızca yerel model kullanılır.</p>
        </>
      )}
      {explanation && <p className="ks-preview" translate="no">{explanation}</p>}
      {command !== null && (
        <div className="ks-form">
          <label htmlFor={`${id}-c`} className="ks-label">Önerilen komut (düzenlenebilir)</label>
          <div className="ks-inline">
            <input id={`${id}-c`} className="text-input" value={command} onChange={(e) => setCommand(e.target.value)} maxLength={8192} spellCheck={false} translate="no" />
            <button type="button" className="btn small danger" onClick={() => void run()} disabled={busy !== null || !command.trim()} data-kor-cmd="shell.runFix">
              <Play size={14} aria-hidden="true" />
              Çalıştır…
            </button>
          </div>
        </div>
      )}
      {result && (
        <div className={result.code === 0 ? 'ks-ok' : 'ks-broken'} role="status">
          {`Çıkış kodu ${result.code}`}
        </div>
      )}
      {result && result.output && <pre className="ks-code" translate="no">{result.output}</pre>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

export function ShellSection() {
  return (
    <div className="ks ks-settings">
      <Integration />
      <LastFailure />
    </div>
  )
}
