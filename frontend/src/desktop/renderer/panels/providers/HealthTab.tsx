// Sağlık: `provider.status` (gecikme, hata oranı, kalan kota) ve sağlayıcı başına `provider.test` (akış dahil).
import { Activity, FlaskConical } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { ProviderTestResult } from '../../../ipc-providers.ts'
import { errorMessage } from '../../api.ts'
import { formatMs } from '../../g7-format.ts'
import { kor } from './bridge.ts'
import { ago } from './catalog-view.ts'
import { ProviderLogo } from './ProviderLogo.tsx'
import { uiLocale, percentText } from '../../ui-locale.ts'

function percent(rate: number | null): string {
  return rate === null ? '—' : percentText(Math.round(rate * 100))
}

function quota(remaining: number | null, limit: number | null): string {
  if (remaining === null) return '—'
  return limit === null ? remaining.toLocaleString(uiLocale()) : `${remaining.toLocaleString(uiLocale())} / ${limit.toLocaleString(uiLocale())}`
}

function TestOutcome({ r }: { r: ProviderTestResult }) {
  if (!r.check.ok) return <span className="error-text">{r.check.error?.message ?? 'Başarısız'}</span>
  // Her parça ayrı düğüm: çeviri kalıpları ("{0} model", "liste {0}") tek tek eşleşsin.
  const parts = [`${r.check.models ?? 0} model`, `liste ${formatMs(r.check.latencyMs)}`]
  if (r.stream?.ok) parts.push(`ilk token ${formatMs(r.stream.firstTokenMs)}`, `toplam ${formatMs(r.stream.totalMs)}`)
  return (
    <span className="kp-ok kp-parts">
      {parts.map((p) => (
        <span key={p}>{p}</span>
      ))}
      {r.stream && !r.stream.ok && <span className="error-text">{`akış hatası: ${r.stream.error?.message ?? ''}`}</span>}
      {r.model && <span translate="no">{r.model}</span>}
    </span>
  )
}

export function HealthTab() {
  const { data, error, isLoading, isValidating, mutate } = useSWR('provider.status', () => kor.provider.status(), { revalidateOnFocus: false })
  const [tests, setTests] = useState<Record<string, ProviderTestResult | string>>({})
  const [busy, setBusy] = useState<string | null>(null)

  const test = async (id: string) => {
    setBusy(id)
    try {
      const r = await kor.provider.test(id)
      setTests((t) => ({ ...t, [id]: r }))
      await mutate()
    } catch (e) {
      setTests((t) => ({ ...t, [id]: errorMessage(e) }))
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="kp-section" aria-labelledby="kp-health-h">
      <div className="kp-head">
        <h3 id="kp-health-h" className="kp-h">
          Sağlayıcı sağlığı
        </h3>
        <button type="button" className="btn small" data-kor-cmd="provider.status" onClick={() => void mutate()} disabled={isValidating}>
          <Activity size={14} aria-hidden="true" /> {isValidating ? 'Yoklanıyor...' : 'Yeniden yokla'}
        </button>
      </div>
      <p className="g7-note">Anahtarı olan her sağlayıcıya model listesi isteği gider; gecikme, hata oranı ve kota son çağrılardan ölçülür.</p>
      {error && (
        <p className="error-text" role="alert">
          {errorMessage(error)}
        </p>
      )}
      {isLoading && <p className="muted">Yoklanıyor...</p>}
      {data && data.length === 0 && <p className="kp-empty">Anahtarı olan sağlayıcı yok. Anahtarlar sekmesinden bir anahtar ekle.</p>}
      {data && data.length > 0 && (
        <ul className="kp-list">
          {data.map((row) => {
            const base = row.provider.split('#')[0] ?? row.provider
            const t = tests[row.provider]
            return (
              <li key={row.provider} className="kp-row">
                <div className="kp-row-main">
                  <ProviderLogo provider={base} />
                  <span className="kp-name" translate="no">
                    {row.provider}
                  </span>
                  <span className={`badge g9-health ${row.lastError ? 'bad' : 'ok'}`}>{formatMs(row.latencyMs)}</span>
                  <span className="kp-grow" />
                  <button type="button" className="btn small" data-kor-cmd="provider.test" onClick={() => void test(row.provider)} disabled={busy !== null}>
                    <FlaskConical size={14} aria-hidden="true" /> {busy === row.provider ? 'Test ediliyor...' : 'Test et'}
                  </button>
                </div>
                <dl className="kp-facts">
                  <dt>Hata oranı</dt>
                  <dd>{percent(row.errorRate)}</dd>
                  <dt>Çağrı</dt>
                  <dd>{row.calls}</dd>
                  <dt>Kalan istek</dt>
                  <dd>{quota(row.remainingRequests, row.limitRequests)}</dd>
                  <dt>Kalan token</dt>
                  <dd>{quota(row.remainingTokens, row.limitTokens)}</dd>
                  {row.updatedAt !== null && (
                    <>
                      <dt>Güncellendi</dt>
                      <dd>{ago(row.updatedAt)}</dd>
                    </>
                  )}
                </dl>
                {row.lastError && <p className="error-text kp-small">{row.lastError}</p>}
                {t && (
                  <p className="kp-small" role="status">
                    {typeof t === 'string' ? <span className="error-text">{t}</span> : <TestOutcome r={t} />}
                  </p>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
