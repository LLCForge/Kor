// Medya işleri (§20): çalışan fal/HF işleri, `kor:media/*` ilerlemesi, iptal; Hugging Face model araması ve indirme.
import { Download, Search, X } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { HfInfo, HfModel } from '../../../ipc-providers.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { kor } from './bridge.ts'
import { ago, formatBytes } from './catalog-view.ts'
import { uiLocale } from '../../ui-locale.ts'

type Line = { text: string; done?: number; total?: number }

function JobsList() {
  const [lines, setLines] = useState<Record<string, Line>>({})
  // Liste gerçek kayıttan gelir; iş varken bitenler düşsün diye kısa aralıkla yeniden okunur.
  const jobs = useSWR('media.jobs', () => kor.media.jobs(), { refreshInterval: (d) => (d && d.length > 0 ? 2000 : 0), revalidateOnFocus: false })
  const [msg, setMsg] = useState<string | null>(null)
  const { mutate } = jobs

  useEffect(() => {
    const offs = [
      kor.media.onProgress((p) => {
        setLines((l) => ({ ...l, [p.jobId]: { text: p.status } }))
        void mutate()
      }),
      kor.media.onHfProgress((p) => {
        setLines((l) => ({ ...l, [p.jobId]: { text: p.file, done: p.done, total: p.total } }))
        void mutate()
      }),
      kor.media.onCancelled(() => void mutate()),
    ]
    return () => offs.forEach((off) => off())
  }, [mutate])

  const cancel = async (jobId?: string) => {
    setMsg(null)
    try {
      await kor.media.cancel(jobId)
      await mutate()
    } catch (e) {
      setMsg(errorMessage(e))
    }
  }

  const list = jobs.data ?? []
  return (
    <div className="kp-stack" data-kor-event="kor:media/progress kor:media/hfProgress kor:media/cancelled">
      <div className="kp-head">
        <h4 className="kp-h4">Çalışan işler</h4>
        {list.length > 1 && (
          <button type="button" className="btn small" data-kor-cmd="media.cancel" onClick={() => void cancel()}>
            Hepsini iptal et
          </button>
        )}
      </div>
      {jobs.error && <p className="error-text">{errorMessage(jobs.error)}</p>}
      {msg && (
        <p className="error-text" role="alert">
          {msg}
        </p>
      )}
      {jobs.data && list.length === 0 && <p className="kp-empty">Çalışan görsel/video üretimi veya model indirmesi yok.</p>}
      {list.length > 0 && (
        <ul className="kp-list" data-kor-cmd="media.jobs">
          {list.map((j) => {
            const l = lines[j.jobId]
            const pct = l?.total ? Math.min(100, Math.round(((l.done ?? 0) * 100) / l.total)) : null
            return (
              <li key={j.jobId} className="kp-row">
                <div className="kp-row-main">
                  <span className="badge neutral">{j.kind === 'hf' ? 'İndirme' : 'Üretim'}</span>
                  <span className="kp-name" translate="no">
                    {j.target}
                  </span>
                  <span className="kp-small">{ago(j.startedAt)}</span>
                  <span className="kp-grow" />
                  <button type="button" className="action-btn" data-kor-cmd="media.cancel" onClick={() => void cancel(j.jobId)} aria-label={`${j.target} işini iptal et`} title="İptal et">
                    <X size={14} aria-hidden="true" />
                  </button>
                </div>
                {l && (
                  <div className="kp-progress" role="status">
                    {pct !== null && <progress max={100} value={pct} aria-label={`${j.target} ilerlemesi`} />}
                    <span className="kp-small" translate="no">
                      {l.text}
                      {l.total ? ` · ${formatBytes(l.done ?? 0)} / ${formatBytes(l.total)}` : ''}
                    </span>
                  </div>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}

function HfSearch() {
  const id = useId()
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<HfModel[] | null>(null)
  const [info, setInfo] = useState<HfInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true)
    setMsg(null)
    try {
      await fn()
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    } finally {
      setBusy(false)
    }
  }

  const search = (e: FormEvent) => {
    e.preventDefault()
    void guard(async () => {
      setInfo(null)
      setResults((await kor.media.hfSearch(query.trim() || undefined)).models)
    })
  }

  const open = (repo: string) => void guard(async () => setInfo(await kor.media.hfInfo(repo)))

  const download = async (repo: string, file: string, size: number | null) => {
    const ok = await confirmAction({
      title: 'Model dosyası indirilsin mi?',
      message: `${repo} / ${file} (${formatBytes(size)}) Kor'un modeller klasörüne indirilip SHA-256 ile doğrulanacak.`,
      confirmLabel: 'İndir',
    })
    if (!ok) return
    // İndirme sürerken iş listesinde görünür; bu düğme bitmesini beklemez.
    kor.media
      .hfDownload(repo, file)
      .then((r) => setMsg({ ok: true, text: `İndirildi: ${r.files.map((f) => `${f.path}${f.verified ? ' (doğrulandı)' : ''}`).join(', ')}` }))
      .catch((e: unknown) => setMsg({ ok: false, text: errorMessage(e) }))
  }

  return (
    <div className="kp-stack">
      <h4 className="kp-h4">Hugging Face modelleri</h4>
      <form className="kp-inline-form" onSubmit={search}>
        <label htmlFor={id} className="sr-only">
          Hugging Face'te ara
        </label>
        <input id={id} className="text-input" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="ör. qwen gguf" maxLength={200} />
        <button type="submit" className="btn small" data-kor-cmd="media.hfSearch" disabled={busy}>
          <Search size={14} aria-hidden="true" /> Ara
        </button>
      </form>
      {msg && (
        <p className={msg.ok ? 'g7-note' : 'error-text'} role="status">
          {msg.text}
        </p>
      )}
      {results && results.length === 0 && <p className="kp-empty">Sonuç yok.</p>}
      {results && results.length > 0 && !info && (
        <ul className="kp-list">
          {results.map((m) => (
            <li key={m.id} className="kp-row-main">
              <button type="button" className="kp-link" data-kor-cmd="media.hfInfo" onClick={() => open(m.id)} disabled={busy} translate="no">
                {m.id}
              </button>
              {m.pipeline && <span className="badge neutral">{m.pipeline}</span>}
              <span className="kp-grow" />
              <span className="kp-small">{`${(m.downloads ?? 0).toLocaleString(uiLocale())} indirme`}</span>
            </li>
          ))}
        </ul>
      )}
      {info && (
        <div className="kp-stack">
          <div className="kp-row-main">
            <span className="kp-name" translate="no">
              {info.id}
            </span>
            {info.license && <span className="badge neutral">{info.license}</span>}
            {info.gated && <span className="badge g9-health bad">Erişim onayı gerekli</span>}
            <span className="kp-grow" />
            <button type="button" className="btn ghost small" onClick={() => setInfo(null)}>
              Sonuçlara dön
            </button>
          </div>
          <ul className="kp-list">
            {info.files.map((f) => (
              <li key={f.path} className="kp-row-main">
                <span className="g7-mono kp-name" translate="no">
                  {f.path}
                </span>
                <span className="kp-small">{formatBytes(f.size)}</span>
                <button type="button" className="action-btn" data-kor-cmd="media.hfDownload" onClick={() => void download(info.id, f.path, f.size)} aria-label={`${f.path} indir`} title="İndir">
                  <Download size={14} aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

export function MediaTab() {
  return (
    <section className="kp-section" aria-labelledby="kp-media-h">
      <h3 id="kp-media-h" className="kp-h">
        Medya ve model indirmeleri
      </h3>
      <JobsList />
      <HfSearch />
    </section>
  )
}
