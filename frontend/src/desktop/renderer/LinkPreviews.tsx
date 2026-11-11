// [g3] Yanıttaki bağlantılar için önizleme kartları.
// Önizleme yalnızca kullanıcı isteyince alınır: model çıktısındaki adrese kendiliğinden istek atmak,
// istem enjeksiyonuyla veri sızdırmanın (adres içine gömülmüş gizli bilgi) kolay bir yolu olurdu.
import { ExternalLink, Globe, LoaderCircle } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from './api.ts'

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

function LinkCard({ url }: { url: string }) {
  const [requested, setRequested] = useState(false)
  const { data, error, isLoading } = useSWR(requested ? ['g3-link', url] : null, () => api.g3LinkPreview(url), {
    revalidateOnFocus: false,
    shouldRetryOnError: false,
  })
  const host = hostOf(url)
  const open = () => {
    api.openExternal(url).catch((err: unknown) => console.warn('Bağlantı açılamadı:', err))
  }

  return (
    <li className="g3-link-card">
      <button type="button" className="g3-link-main" onClick={open} title={url} aria-label={`${host} bağlantısını tarayıcıda aç`}>
        <Globe size={14} aria-hidden="true" className="g3-link-icon" />
        <span className="g3-link-text">
          <span className="g3-link-site">{data?.siteName ?? host}</span>
          {data?.title ? <span className="g3-link-title">{data.title}</span> : <span className="g3-link-url">{url}</span>}
          {data?.description && <span className="g3-link-desc">{data.description}</span>}
        </span>
        <ExternalLink size={13} aria-hidden="true" className="g3-link-icon" />
      </button>
      {!requested && (
        <button type="button" className="btn small ghost g3-link-load" onClick={() => setRequested(true)}>
          Önizle
        </button>
      )}
      {isLoading && <LoaderCircle size={14} className="spin g3-link-load" aria-label="Önizleme yükleniyor" />}
      {error && (
        <span className="g3-link-note error-text" role="alert">
          {errorMessage(error)}
        </span>
      )}
      {requested && !isLoading && !error && data === null && <span className="g3-link-note">Önizleme bilgisi yok</span>}
    </li>
  )
}

export function LinkPreviews({ urls }: { urls: readonly string[] }) {
  if (urls.length === 0) return null
  return (
    <ul className="g3-links" aria-label="Yanıttaki bağlantılar">
      {urls.map((url) => (
        <LinkCard key={url} url={url} />
      ))}
    </ul>
  )
}
