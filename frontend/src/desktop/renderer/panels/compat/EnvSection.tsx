// Uyumluluk panelindeki ortam bölümü (§15.7, ADR 0043): algılanan devcontainer / compose / nix / mise
// dosyaları, ajan komutlarının nasıl çalıştığı, sandbox ile birleşimi ve beklenen/bulunan sürümler.
// CLI: kor env status. Sürüm yoklaması bir komut çalıştırdığı için yalnızca düğmeyle başlar.
import { RefreshCw } from 'lucide-react'
import { useId } from 'react'
import useSWR from 'swr'
import type { EnvStatus, EnvVersion } from '../../../ipc-replay.ts'
import { errorMessage } from '../../api.ts'
import { envApi } from '../replay/api.ts'
import '../security/panel.css'
import '../replay/panel.css'

const SANDBOX: Record<EnvStatus['sandbox'], string> = {
  inside: 'Sarmalayıcı OS sandbox’ı içinde çalışır.',
  asks: 'Konteyner ortamı OS sandbox’ı ile sınırlanamaz: her komut önce onay ister.',
  image: 'Docker sandbox imajı ortamın kendisidir; projenin ortam dosyaları kullanılmaz.',
  none: 'Sandbox kapalı.',
}

function state(v: EnvVersion): string {
  if (v.ok === true) return 'uygun'
  if (v.ok === false || (v.found === null && /^v?\d/.test(v.expected))) return 'uyuşmuyor'
  return 'karşılaştırılamaz'
}

export function EnvSection() {
  const id = useId()
  const { data, error, isLoading, mutate } = useSWR('env:status', () => envApi.status(), { revalidateOnFocus: false })
  return (
    <section className="g14-section" aria-labelledby={`${id}-env`} data-kor-cmd="env.status">
      <div className="kmem-row">
        <h3 className="g14-h" id={`${id}-env`}>Geliştirme ortamı</h3>
        <button type="button" className="action-btn" onClick={() => void mutate()} disabled={isLoading} aria-label="Ortamı ve sürümleri yeniden denetle" title="Yeniden denetle" data-kor-cmd="env.status">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">Ortam dosyaları ve araç sürümleri denetleniyor…</p>
      ) : !data.enabled ? (
        <p className="g7-note">Ortam farkındalığı kapalı (<code>[env] enabled = false</code>).</p>
      ) : data.detected.files.length === 0 ? (
        <p className="g7-note">Bu projede devcontainer, Nix, compose, mise/asdf ya da sürüm dosyası yok; komutlar doğrudan bu makinede çalışır.</p>
      ) : (
        <>
          <p className="g7-note" translate="no">{data.detected.files.join(', ')}</p>
          {data.detected.kind && (
            <p className="g7-note">
              Ortam: <strong translate="no">{data.detected.kind.kind}</strong>
              {data.example && <> · komutlar: <span className="kr-env-cmd" translate="no">{data.example}</span></>}
            </p>
          )}
          <p className="g7-note">{SANDBOX[data.sandbox]} ({data.sandboxBackend})</p>
          {data.detected.dockerfile_dev && <p className="ks-warn">Dockerfile.dev bulundu: Kor kendiliğinden imaj derlemez; docker-compose.dev.yml ya da devcontainer ekleyin.</p>}
          {data.toolMissing && data.reason && <p className="ks-warn" translate="no">{data.reason}</p>}
          {data.versions.length > 0 && (
            <table className="core-table">
              <thead>
                <tr>
                  <th scope="col">Araç</th>
                  <th scope="col">Beklenen</th>
                  <th scope="col">Bulunan</th>
                  <th scope="col">Durum</th>
                </tr>
              </thead>
              <tbody>
                {data.versions.map((v) => (
                  <tr key={`${v.tool}:${v.source}:${v.expected}`}>
                    <th scope="row">{v.tool}</th>
                    <td translate="no">{v.expected} <span className="muted">({v.source})</span></td>
                    <td translate="no">{v.found ?? '—'}</td>
                    <td>{state(v)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {data.problems ? (
            <p className={data.onMismatch === 'stop' ? 'ks-broken' : 'ks-warn'} role="status">
              {data.onMismatch === 'stop' ? 'Uyuşmazlık giderilene kadar ajan komutları reddedilir' : 'Ajan komutları uyarıyla çalışır'} (<code>[env] on_mismatch = "{data.onMismatch}"</code>).
            </p>
          ) : (
            data.versions.length > 0 && <p className="ks-ok">Sabitlenmiş sürümler uyuyor.</p>
          )}
        </>
      )}
    </section>
  )
}
