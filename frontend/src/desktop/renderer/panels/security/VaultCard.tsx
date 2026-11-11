// Sır kasası (§6.6): yalnızca adlar listelenir. Değer bir kez gönderilir, anahtarlığa yazılır ve arayüze hiç dönmez.
import { FileUp, KeyRound, Plus, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { VaultImport } from '../../../ipc-security.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { vaultApi } from './bridge.ts'
import { scopeLabel, validVaultName } from './shape.ts'

const VAULT_KEY = 'security:vault'

function AddForm({ onDone }: { onDone: (names: string[]) => void }) {
  const id = useId()
  const [name, setName] = useState('')
  const [value, setValue] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nameOk = validVaultName(name.trim())

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!nameOk || !value) return
    setBusy(true)
    setError(null)
    try {
      const res = await vaultApi.add(name.trim(), value)
      setName('')
      setValue('')
      onDone(res.names)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="ks-form" onSubmit={(e) => void submit(e)}>
      <div className="ks-field">
        <label htmlFor={`${id}-n`}>Ad</label>
        <input id={`${id}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="DATABASE_URL" maxLength={128} autoComplete="off" spellCheck={false} translate="no" aria-invalid={name !== '' && !nameOk} />
      </div>
      <div className="ks-field">
        <label htmlFor={`${id}-v`}>Değer</label>
        <input id={`${id}-v`} className="text-input" type="password" value={value} onChange={(e) => setValue(e.target.value)} maxLength={8192} autoComplete="off" spellCheck={false} />
      </div>
      {name !== '' && !nameOk && <p className="ks-warn">Harf, rakam ve _ kullanın; rakamla başlamasın.</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      <button type="submit" className="btn small primary" disabled={busy || !nameOk || !value} data-kor-cmd="vault.add">
        <Plus size={14} aria-hidden="true" />
        {busy ? 'Kaydediliyor…' : 'Kasaya ekle'}
      </button>
    </form>
  )
}

function ImportForm({ onDone }: { onDone: (names: string[]) => void }) {
  const id = useId()
  const [path, setPath] = useState('')
  const [preview, setPreview] = useState<VaultImport | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)

  const look = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    setDone(null)
    try {
      setPreview(await vaultApi.import(path.trim() || null, false))
    } catch (err) {
      setPreview(null)
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const run = async () => {
    if (!preview || preview.found.length === 0) return
    const ok = await confirmAction({
      title: '.env içe aktarılsın mı?',
      message: preview.existing.length
        ? `${preview.path} dosyasındaki ${preview.found.length} sır kasaya yazılacak; şu adların mevcut değeri değişecek: ${preview.existing.join(', ')}. Dosya değiştirilmez.`
        : `${preview.path} dosyasındaki ${preview.found.length} sır kasaya yazılacak. Dosya değiştirilmez.`,
      confirmLabel: 'İçe aktar',
      danger: preview.existing.length > 0,
    })
    if (!ok) return
    setBusy(true)
    setError(null)
    try {
      const res = await vaultApi.import(path.trim() || null, true)
      setPreview(null)
      setDone(`${res.found.length} sır kasaya kaydedildi.`)
      onDone(res.names)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="ks-form">
      <form className="ks-inline" onSubmit={(e) => void look(e)}>
        <label htmlFor={`${id}-p`} className="sr-only">.env yolu</label>
        <input id={`${id}-p`} className="text-input" value={path} onChange={(e) => { setPath(e.target.value); setPreview(null) }} placeholder=".env (proje kökü)" maxLength={1024} spellCheck={false} translate="no" />
        <button type="submit" className="btn small" disabled={busy} data-kor-cmd="vault.import">
          <FileUp size={14} aria-hidden="true" />
          Önizle
        </button>
      </form>
      {preview && (
        <div className="ks-preview" role="status">
          {preview.found.length === 0 ? (
            <p className="muted">{`${preview.path} içinde içe aktarılacak KEY=değer satırı yok.`}</p>
          ) : (
            <>
              <p>{`${preview.path}: ${preview.found.length} sır`}</p>
              <p className="ks-names" translate="no">{preview.found.join(', ')}</p>
            </>
          )}
          {preview.skipped.length > 0 && <p className="muted ks-note">Atlanan: <span translate="no">{preview.skipped.join(', ')}</span></p>}
          {preview.found.length > 0 && (
            <button type="button" className="btn small primary" onClick={() => void run()} disabled={busy} data-kor-cmd="vault.import">
              İçe aktar…
            </button>
          )}
        </div>
      )}
      {done && <p className="muted" role="status">{done}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

export function VaultCard() {
  const id = useId()
  const { data, error, mutate } = useSWR(VAULT_KEY, () => vaultApi.list())
  const [actionError, setActionError] = useState<string | null>(null)
  const setNames = (names: string[]) => void mutate(data ? { ...data, names } : undefined, { revalidate: !data })

  const remove = async (name: string) => {
    const ok = await confirmAction({ title: `${name} silinsin mi?`, message: 'Değer kasadan kalıcı olarak silinir; geri alınamaz.', confirmLabel: 'Sil', danger: true })
    if (!ok) return
    setActionError(null)
    try {
      setNames((await vaultApi.remove(name)).names)
    } catch (err) {
      setActionError(errorMessage(err))
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <KeyRound size={16} aria-hidden="true" />
        Sır kasası
      </h3>
      <p className="muted ks-note">
        {data?.backend === 'file' ? (
          <>Sırlar parolayla şifrelenmiş yerel dosyada tutulur (<code translate="no">{data.file ?? 'vault.enc'}</code>; parola KOR_VAULT_PASSPHRASE ortam değişkeninden okunur).</>
        ) : (
          'Sırlar işletim sisteminin anahtarlığında tutulur.'
        )}{' '}
        Ajanın çalıştırdığı komutlara ortam değişkeni olarak verilir; modele giden her istekte ve çıktılarda maskelenir. Hiçbir dosyaya ya da isteme yazılmaz, değeri gösteren bir düğme bilerek yoktur.
      </p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="vault.list">Kasa okunuyor…</p>
      ) : data.names.length === 0 ? (
        <p className="muted ks-empty" data-kor-cmd="vault.list">Kasa boş. Aşağıdan bir sır ekleyin ya da bir .env dosyasını içe aktarın.</p>
      ) : (
        <ul className="ks-list" aria-label="Saklanan sırlar" data-kor-cmd="vault.list">
          {data.names.map((n) => (
            <li key={n} className="ks-row">
              <code translate="no">{n}</code>
              <span className="muted" aria-hidden="true">••••••</span>
              <span className="muted ks-note" title="[vault.scope] ayarı: bu sırrı alan komutlar">
                {'→ '}
                <span translate={data.scope[n] ? 'no' : undefined}>{scopeLabel(data.scope[n])}</span>
              </span>
              <button type="button" className="action-btn" onClick={() => void remove(n)} aria-label={`${n} sırrını sil`} title="Sil" data-kor-cmd="vault.remove">
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
      <details className="ks-more">
        <summary>Sır ekle</summary>
        <AddForm onDone={setNames} />
      </details>
      <details className="ks-more">
        <summary>.env içe aktar</summary>
        <ImportForm onDone={setNames} />
      </details>
    </section>
  )
}
