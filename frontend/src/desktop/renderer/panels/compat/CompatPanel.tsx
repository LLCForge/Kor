// Uyumluluk paneli (§3.3): diğer araçların bulunan izleri, yüklenen kurallar (kaynak + öncelik), çakışmalar,
// diğer araçlarda tanımlı MCP sunucuları ve Kor kurallarını dışa aktarma. Kaynaklar tıklanınca dosya yöneticisinde
// gösterilir (yalnızca Kor'un yüklediği yollar). Kural metinleri arayüze gelmez. CLI: kor compat status|conflicts|export.
import { FileOutput } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { CompatExportTarget, CompatSource } from '../../../ipc-compat.ts'
import { api, errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { compatApi } from './api.ts'
import { EnvSection } from './EnvSection.tsx'
import { appliesText, ecoName, shortPath, toolSummary } from './shape.ts'

const SCOPE: Record<CompatSource['scope'], string> = { user: 'kullanıcı', project: 'proje', local: 'yerel' }

function Source({ s, onError }: { s: CompatSource; onError: (e: unknown) => void }) {
  return (
    <button
      type="button"
      className="kmem-link g7-mono"
      title={`${s.path} (dosya yöneticisinde göster)`}
      onClick={() => void compatApi.reveal(s.path).catch(onError)}
      data-kor-cmd="compat.reveal"
      translate="no"
    >
      {shortPath(s.path)}
    </button>
  )
}

function Loading({ error }: { error: unknown }) {
  return error ? <p className="error-text" role="alert">{errorMessage(error)}</p> : <p className="muted">Yükleniyor...</p>
}

export function CompatPanel(_props: KorPanelProps) {
  const id = useId()
  const status = useSWR('core:compat', () => api.core.compat(), { revalidateOnFocus: false })
  const conflicts = useSWR('compat:conflicts', () => compatApi.conflicts(), { revalidateOnFocus: false })
  const mcp = useSWR('compat:mcp', () => compatApi.mcp(), { revalidateOnFocus: false })
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const fail = (e: unknown) => setNote({ text: errorMessage(e), ok: false })

  const exportTo = (target: CompatExportTarget) => {
    setBusy(true)
    setNote(null)
    compatApi
      .export(target)
      .then((r) =>
        setNote({
          text: `${r.rules} kural ${r.files.length} dosyaya yazıldı: ${r.dir}` + (r.warnings.length ? ` · ${r.warnings.join(' · ')}` : ''),
          ok: true,
        }),
      )
      .catch(fail)
      .finally(() => setBusy(false))
  }

  const d = status.data
  const p = d?.permissions
  const others = d && p
    ? [
        ...p.allow,
        ...p.ask,
        ...p.deny,
        ...(p.mode ? [p.mode] : []),
        ...d.hooks,
        ...d.commands,
        ...d.agents,
        ...d.skills,
        ...(mcp.data ?? []),
      ]
    : []
  const tools = d ? toolSummary(d.instructions, others) : []
  const permRows = p ? [...p.allow.map((r) => ({ ...r, kind: 'izin' })), ...p.ask.map((r) => ({ ...r, kind: 'sor' })), ...p.deny.map((r) => ({ ...r, kind: 'yasak' }))] : []

  return (
    <div className="kmem-panel">
      <div className="g14-body">
        <section className="g14-section" aria-labelledby={`${id}-tools`} data-kor-cmd="core.compat">
          <h3 className="g14-h" id={`${id}-tools`}>Bulunan araçlar</h3>
          {!d ? (
            <Loading error={status.error} />
          ) : (
            <>
              <p className="g7-note">
                {d.trusted ? 'Proje güvenilir: proje ayarları izin verebilir.' : 'Proje güvenilir değil: proje ayarları yalnızca kısıtlayabilir.'}
              </p>
              {tools.length === 0 ? (
                <p className="g7-note">Bu projede başka bir aracın izi yok.</p>
              ) : (
                <table className="core-table">
                  <thead>
                    <tr>
                      <th scope="col">Araç</th>
                      <th scope="col">Dosya</th>
                      <th scope="col">Kural</th>
                      <th scope="col">Diğer tanım</th>
                    </tr>
                  </thead>
                  <tbody>
                    {tools.map((t) => (
                      <tr key={t.eco}>
                        <th scope="row">{ecoName(t.eco)}</th>
                        <td>{t.files}</td>
                        <td>{t.rules}</td>
                        <td>{t.other}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
              {[...d.warnings, ...d.untrustedSkipped].map((w) => (
                <p key={w} className="error-text" translate="no">
                  {w}
                </p>
              ))}
            </>
          )}
        </section>

        <section className="g14-section" aria-labelledby={`${id}-rules`}>
          <h3 className="g14-h" id={`${id}-rules`}>Yüklenen kurallar</h3>
          <p className="g7-note">Birleştirme sırasıyla: alttaki kural üsttekinden önceliklidir (genel → AGENTS.md → araç dosyaları → glob kuralları → KOR.md ve .kor/rules).</p>
          {!d ? (
            <Loading error={status.error} />
          ) : d.instructions.length === 0 ? (
            <p className="g7-note">Yüklenen talimat dosyası yok.</p>
          ) : (
            <ol className="core-list">
              {d.instructions.map((i, n) => (
                <li key={i.source.path} className="core-row">
                  <span className="kmem-tier g7-mono" title="Öncelik (büyük olan kazanır)">{n + 1}</span>
                  <span className="core-row-main">
                    <Source s={i.source} onError={fail} />
                    <br />
                    <span className="g7-note">
                      {ecoName(i.source.eco)} · {SCOPE[i.source.scope]} · {appliesText(i.applies)} · {i.chars} karakter
                    </span>
                  </span>
                </li>
              ))}
            </ol>
          )}
          {permRows.length > 0 && (
            <table className="core-table" aria-label="İzin kuralları">
              <tbody>
                {permRows.map((r, n) => (
                  <tr key={`${r.kind}:${r.rule}:${n}`}>
                    <th scope="row">{r.kind}</th>
                    <td>
                      <code translate="no">{r.rule}</code>
                    </td>
                    <td>
                      <Source s={r.source} onError={fail} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>

        <section className="g14-section" aria-labelledby={`${id}-conf`} data-kor-cmd="compat.conflicts">
          <h3 className="g14-h" id={`${id}-conf`}>Çakışmalar</h3>
          <p className="g7-note">Farklı araçlarda aynı kural (aynı glob ya da dosya adı) farklı metinle. Hiçbiri atılmaz; kazanan en son okunur.</p>
          {!conflicts.data ? (
            <Loading error={conflicts.error} />
          ) : conflicts.data.length === 0 ? (
            <p className="g7-note">Çakışma yok.</p>
          ) : (
            <ul className="core-list">
              {conflicts.data.map((c, n) => (
                <li key={n} className="kmem-card">
                  <code translate="no">{c.key.kind === 'globs' ? c.key.globs.join(', ') : c.key.name}</code>
                  {c.rules.map((r) => (
                    <span key={r.path} className="kmem-row">
                      <Source s={r} onError={fail} />
                      <span className={r.path === c.winner ? 'kmem-winner' : 'muted'}>
                        {ecoName(r.eco)}
                        {r.path === c.winner && ' · kazanan'}
                      </span>
                    </span>
                  ))}
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="g14-section" aria-labelledby={`${id}-mcp`} data-kor-cmd="compat.mcp">
          <h3 className="g14-h" id={`${id}-mcp`}>Diğer araçlardaki MCP sunucuları</h3>
          <p className="g7-note">Yalnızca adları okunur; komut, adres ve ortam değişkenleri sır içerebileceği için saklanmaz.</p>
          {!mcp.data ? (
            <Loading error={mcp.error} />
          ) : mcp.data.length === 0 ? (
            <p className="g7-note">Tanımlı MCP sunucusu yok.</p>
          ) : (
            <ul className="core-list">
              {mcp.data.map((m) => (
                <li key={`${m.name}:${m.source.path}`} className="core-row">
                  <strong className="core-row-main" translate="no">{m.name}</strong>
                  <span className="muted">{ecoName(m.source.eco)}</span>
                  <Source s={m.source} onError={fail} />
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="g14-section" aria-labelledby={`${id}-exp`} data-kor-cmd="compat.export">
          <h3 className="g14-h" id={`${id}-exp`}>Dışa aktar</h3>
          <p className="g7-note">Kor'un kendi kuralları (KOR.md, .kor/rules) diğer araçların biçimine; yalnızca .kor/export/ altına yazılır, araçların dosyalarına dokunulmaz.</p>
          <div className="kmem-row kmem-wrap">
            {(['cursor', 'claude', 'agents'] as const).map((t) => (
              <button key={t} type="button" className="btn small" disabled={busy} onClick={() => exportTo(t)}>
                <FileOutput size={14} aria-hidden="true" /> {t === 'cursor' ? 'Cursor' : t === 'claude' ? 'Claude Code' : 'AGENTS.md'}
              </button>
            ))}
          </div>
        </section>
        <EnvSection />
        {note && (
          <p className={note.ok ? 'g7-note' : 'error-text'} role={note.ok ? 'status' : 'alert'}>
            {note.text}
          </p>
        )}
      </div>
    </div>
  )
}
