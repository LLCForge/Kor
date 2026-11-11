// [g13] Ortam değişkeni yöneticisi: kökteki .env dosyaları. Değerler varsayılan maskeli, her biri ancak "Göster" ile
// ana süreçten alınır ve yalnızca bu bileşenin durumunda tutulur (günlüğe yazılmaz, ajana gitmez).
import { Eye, EyeOff, Pencil, Plus, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'

const KEY_PATTERN = /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/
const MASK = '••••••••'

const templateKey = (root: string) => ['g13:env:template', root] as const

/** Şablon (.env.example) ile karşılaştırma: eksik/boş değişken uyarısı; şablon yoksa .env'den üretme düğmesi. */
function EnvTemplateBanner({ root, hasEnv }: { root: string; hasEnv: boolean }) {
  const { data: report, mutate } = useSWR(templateKey(root), () => api.g13.envTemplateReport())
  const { mutate: globalMutate } = useSWRConfig()
  const [status, setStatus] = useState<string | null>(null)
  if (!report) return null
  if (!report.template) {
    if (!hasEnv) return null
    const create = () =>
      api.g13
        .envCreateTemplate()
        .then(async (name) => {
          setStatus(`${name} oluşturuldu: değerler silindi, yalnızca değişken adları var. Depoya eklenebilir.`)
          await Promise.all([mutate(), globalMutate(['g13:env:files', root])])
        })
        .catch((err: unknown) => setStatus(errorMessage(err)))
    return (
      <div className="g31-env-banner">
        <span>Şablon yok. .env.example, ekip arkadaşlarının hangi değişkenlerin gerektiğini görmesini sağlar.</span>
        <button type="button" className="btn small" onClick={() => void create()}>
          Şablon oluştur
        </button>
        {status && <p className="g7-note" role="status">{status}</p>}
      </div>
    )
  }
  const problems = report.missing.length + report.empty.length
  return (
    <div className={`g31-env-banner${problems > 0 ? ' warn' : ' ok'}`} role={problems > 0 ? 'alert' : 'status'}>
      {problems === 0 ? (
        <span>{report.template} ile karşılaştırıldı: tüm değişkenler tanımlı.</span>
      ) : (
        <span>
          {report.template} ile karşılaştırıldı.
          {report.missing.length > 0 && <> Eksik: <code>{report.missing.join(', ')}</code>.</>}
          {report.empty.length > 0 && <> Boş: <code>{report.empty.join(', ')}</code>.</>}
        </span>
      )}
      {report.extra.length > 0 && <span className="muted"> Şablonda olmayan: {report.extra.join(', ')}.</span>}
    </div>
  )
}

function EnvTable({ root, file }: { root: string; file: string }) {
  const { data: keys, error, mutate: mutateKeys } = useSWR(['g13:env:keys', root, file], () => api.g13.envKeys(file))
  const { mutate: globalMutate } = useSWRConfig()
  // Değişken eklenip silinince şablon karşılaştırması da yenilenir.
  const mutate = () => Promise.all([mutateKeys(), globalMutate(templateKey(root))])
  const [revealed, setRevealed] = useState<Record<string, string>>({})
  const [editing, setEditing] = useState<{ key: string; value: string } | null>(null)
  const [adding, setAdding] = useState({ key: '', value: '' })
  const [status, setStatus] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const addId = useId()

  const run = async (fn: () => Promise<void>) => {
    setBusy(true)
    setStatus(null)
    try {
      await fn()
    } catch (err) {
      setStatus(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const hide = (key: string) =>
    setRevealed((prev) => {
      const next = { ...prev }
      delete next[key]
      return next
    })

  const reveal = (key: string) =>
    run(async () => {
      const value = await api.g13.envReveal(file, key)
      setRevealed((prev) => ({ ...prev, [key]: value }))
    })

  const save = (key: string, value: string) =>
    run(async () => {
      await api.g13.envSet(file, key, value)
      setEditing(null)
      // Gösterilmekte olan değer güncel kalsın; gizliyse gizli kalır.
      setRevealed((prev) => (key in prev ? { ...prev, [key]: value } : prev))
      await mutate()
    })

  const remove = (key: string) =>
    run(async () => {
      const ok = await confirmAction({ title: 'Değişkeni sil', message: `${key} değişkeni ${file} dosyasından silinsin mi?`, confirmLabel: 'Sil', danger: true })
      if (!ok) return
      await api.g13.envRemove(file, key)
      hide(key)
      await mutate()
    })

  const add = (e: FormEvent) => {
    e.preventDefault()
    const key = adding.key.trim()
    if (!KEY_PATTERN.test(key)) return setStatus('Geçersiz değişken adı: harf veya _ ile başlamalı; harf, rakam, _ . - kullanılabilir.')
    if (keys?.some((k) => k.key === key)) return setStatus(`${key} zaten var; satırındaki Düzenle ile değiştir.`)
    void run(async () => {
      await api.g13.envSet(file, key, adding.value)
      setAdding({ key: '', value: '' })
      await mutate()
    })
  }

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!keys) return <p className="muted">Yükleniyor...</p>

  return (
    <div className="g13-stack">
      {keys.length === 0 ? (
        <p className="muted">{file} içinde değişken yok.</p>
      ) : (
        <div className="g7-table-wrap">
          <table className="g7-table g13-env-table">
            <thead>
              <tr>
                <th scope="col">Ad</th>
                <th scope="col">Değer</th>
                <th scope="col"><span className="sr-only">İşlemler</span></th>
              </tr>
            </thead>
            <tbody>
              {keys.map(({ key, empty }) => {
                const shown = revealed[key]
                const isEditing = editing?.key === key
                return (
                  <tr key={key}>
                    <th scope="row"><code>{key}</code></th>
                    <td className="g13-env-value">
                      {isEditing ? (
                        <form
                          className="g13-row"
                          onSubmit={(e) => {
                            e.preventDefault()
                            void save(key, editing.value)
                          }}
                        >
                          <label className="sr-only" htmlFor={`${addId}-${key}`}>{key} değeri</label>
                          <input
                            id={`${addId}-${key}`}
                            className="text-input g13-grow"
                            type={shown === undefined ? 'password' : 'text'}
                            value={editing.value}
                            onChange={(e) => setEditing({ key, value: e.target.value })}
                            autoComplete="off"
                            spellCheck={false}
                            autoFocus
                          />
                          <button type="submit" className="btn small primary" disabled={busy}>Kaydet</button>
                          <button type="button" className="btn small ghost" onClick={() => setEditing(null)}>İptal</button>
                        </form>
                      ) : shown !== undefined ? (
                        <code className="g13-env-shown">{shown || '(boş)'}</code>
                      ) : (
                        <span className="muted" aria-label={empty ? 'boş değer' : 'gizli değer'}>{empty ? '(boş)' : MASK}</span>
                      )}
                    </td>
                    <td className="g13-env-actions">
                      {!isEditing && (
                        <>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() => (shown === undefined ? void reveal(key) : hide(key))}
                            disabled={busy || empty}
                            aria-label={shown === undefined ? `${key} değerini göster` : `${key} değerini gizle`}
                            title={shown === undefined ? 'Göster' : 'Gizle'}
                          >
                            {shown === undefined ? <Eye size={16} aria-hidden="true" /> : <EyeOff size={16} aria-hidden="true" />}
                          </button>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() => setEditing({ key, value: shown ?? '' })}
                            disabled={busy}
                            aria-label={`${key} değerini düzenle`}
                            title={shown === undefined ? 'Yeni değer yaz (mevcut değeri görmek için önce Göster)' : 'Düzenle'}
                          >
                            <Pencil size={16} aria-hidden="true" />
                          </button>
                          <button type="button" className="action-btn" onClick={() => void remove(key)} disabled={busy} aria-label={`${key} değişkenini sil`} title="Sil">
                            <Trash2 size={16} aria-hidden="true" />
                          </button>
                        </>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}

      <form className="g13-row" onSubmit={add} aria-label="Yeni değişken ekle">
        <label className="sr-only" htmlFor={`${addId}-key`}>Değişken adı</label>
        <input
          id={`${addId}-key`}
          className="text-input"
          placeholder="AD"
          value={adding.key}
          onChange={(e) => setAdding((a) => ({ ...a, key: e.target.value }))}
          autoComplete="off"
          spellCheck={false}
        />
        <label className="sr-only" htmlFor={`${addId}-value`}>Değer</label>
        <input
          id={`${addId}-value`}
          className="text-input g13-grow"
          type="password"
          placeholder="değer"
          value={adding.value}
          onChange={(e) => setAdding((a) => ({ ...a, value: e.target.value }))}
          autoComplete="off"
          spellCheck={false}
        />
        <button type="submit" className="btn small" disabled={busy || !adding.key.trim()}>
          <Plus size={14} aria-hidden="true" /> Ekle
        </button>
      </form>
      {status && <p className="error-text" role="alert">{status}</p>}
    </div>
  )
}

export function EnvPanel({ root }: { root: string }) {
  const { data: files, error } = useSWR(['g13:env:files', root], () => api.g13.envFiles())
  const [file, setFile] = useState('.env')
  const selectId = useId()
  // .env yoksa da seçilebilir: ilk değişken eklenince dosya oluşur.
  const options = [...new Set(['.env', ...(files ?? [])])]

  return (
    <div className="g13-stack">
      <p className="dialog-desc text-pretty">
        Kök klasördeki .env dosyaları. Değerler maskelidir; tek tek Göster ile açılır. Ajan bu dosyaları okuyamaz ve buradaki
        değerler hiçbir zaman sohbete, günlüğe veya denetim kaydına yazılmaz.
      </p>
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      <EnvTemplateBanner root={root} hasEnv={files?.includes('.env') ?? false} />
      <div className="g13-row">
        <label htmlFor={selectId}>Dosya</label>
        <select id={selectId} className="text-input" value={file} onChange={(e) => setFile(e.target.value)}>
          {options.map((f) => (
            <option key={f} value={f}>
              {f}
              {files && !files.includes(f) ? ' (yeni)' : ''}
            </option>
          ))}
        </select>
      </div>
      <EnvTable key={`${root}:${file}`} root={root} file={file} />
    </div>
  )
}
