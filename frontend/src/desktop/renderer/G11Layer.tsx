// [g11] Hafıza katmanları penceresi (oturum / proje bilgi tabanı / global), olası çakışmalar ve güven puanı ayarı.
import { Layers, Pencil, Trash2 } from 'lucide-react'
import { useId, useState, useSyncExternalStore, type ReactNode } from 'react'
import useSWR from 'swr'
import type { MemoryRecord } from '../../core/memory-g11.ts'
import { api, errorMessage } from './api.ts'
import { confirmAction } from './ConfirmDialog.tsx'
import { G7Modal } from './G7Modal.tsx'
import { closeG11Panel, isG11PanelOpen, openG11Panel, subscribeG11Panel } from './g11-bus.ts'
import './G11Confidence.tsx'

const KIND_LABELS = { decision: 'Karar', constraint: 'Kısıt', term: 'Terim', note: 'Not' } as const
const LABEL = 'Hafıza katmanları'

const recordText = (r: MemoryRecord) => (r.subject ? `${r.subject}: ${r.text}` : r.text)

/** Kenar çubuğu alt düğmesi; Sidebar'daki RailButton ile aynı görünüm. */
export function G11RailButton({ collapsed }: { collapsed: boolean }) {
  return (
    <button type="button" className="rail-btn" onClick={openG11Panel} title={collapsed ? LABEL : undefined} aria-label={collapsed ? LABEL : undefined}>
      <Layers size={16} aria-hidden="true" />
      <span className="rail-btn-label">{LABEL}</span>
    </button>
  )
}

interface SectionProps {
  title: string
  desc: string
  records: readonly MemoryRecord[]
  empty: string
  conflicted: ReadonlySet<string>
  busy: string | null
  onForget: (record: MemoryRecord) => void
  action?: ReactNode
}

function LayerSection({ title, desc, records, empty, conflicted, busy, onForget, action }: SectionProps) {
  const headingId = useId()
  return (
    <section className="g11-section" aria-labelledby={headingId}>
      <div className="g11-section-head">
        <div>
          <h3 id={headingId} className="g7-h4">
            {title} <span className="g11-count">{records.length}</span>
          </h3>
          <p className="g7-note">{desc}</p>
        </div>
        {action}
      </div>
      {records.length === 0 ? (
        <p className="g7-note">{empty}</p>
      ) : (
        <ul className="g11-records">
          {records.map((r) => (
            <li key={r.id} className={`g11-record${conflicted.has(r.id) ? ' g11-conflicted' : ''}`}>
              {r.kind && <span className="g11-kind">{KIND_LABELS[r.kind]}</span>}
              <span className="g11-record-text">
                {r.subject && <strong>{r.subject}: </strong>}
                {r.text}
                {conflicted.has(r.id) && <span className="g11-flag"> (çakışma)</span>}
              </span>
              <button
                type="button"
                className="action-btn"
                onClick={() => onForget(r)}
                disabled={busy === r.id}
                aria-label={`Sil: ${recordText(r).slice(0, 80)}`}
                title="Sil"
              >
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

export function G11Layer({ sessionId, onOpenGlobalEditor }: { sessionId: string; onOpenGlobalEditor: () => void }) {
  const open = useSyncExternalStore(subscribeG11Panel, isG11PanelOpen)
  const titleId = useId()
  const toggleId = useId()
  const { data, error, mutate } = useSWR(open ? ['g11:overview', sessionId] : null, () => api.g11GetOverview(sessionId))
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const run = async (key: string, action: () => Promise<void>) => {
    setBusy(key)
    setActionError(null)
    try {
      await action()
      await mutate()
    } catch (err) {
      console.error('g11:', err)
      setActionError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const forget = async (record: MemoryRecord) => {
    const ok = await confirmAction({ title: 'Kaydı sil', message: `"${recordText(record).slice(0, 200)}" hafızadan silinsin mi?`, confirmLabel: 'Sil', danger: true })
    if (ok) await run(record.id, () => api.g11Forget(sessionId, record.id))
  }

  const all = data ? [...data.session, ...(data.project?.records ?? []), ...data.global] : []
  const byId = new Map(all.map((r) => [r.id, r]))
  const label = (id: string) => {
    const r = byId.get(id)
    return r ? recordText(r) : id
  }
  const conflicted = new Set(data?.conflicts.flatMap((c) => [c.a, c.b]) ?? [])
  const project = data?.project ?? null
  const shared = { conflicted, busy, onForget: (r: MemoryRecord) => void forget(r) }

  return (
    <G7Modal open={open} onClose={closeG11Panel} titleId={titleId} title={LABEL} className="g11-modal">
      <div className="g11-body">
        {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
        {actionError && <p className="error-text" role="alert">{actionError}</p>}
        {!data && !error && <p className="muted">Yükleniyor...</p>}
        {data && (
          <>
            <div className="g11-toggle">
              <input
                id={toggleId}
                type="checkbox"
                checked={data.confidenceScore}
                disabled={busy === 'confidence'}
                onChange={(e) => {
                  const enabled = e.target.checked
                  void run('confidence', () => api.g11SetConfidence(enabled))
                }}
              />
              <label htmlFor={toggleId}>
                <span>Yanıt sonunda güven puanı</span>
                <span className="g7-note">Ajan yanıtın sonuna düşük/orta/yüksek emin olma düzeyi ve kısa gerekçe ekler. Bir sonraki turdan itibaren geçerli.</span>
              </label>
            </div>

            {data.conflicts.length > 0 && (
              <section className="g11-section g11-conflicts" aria-label="Olası çakışmalar">
                <h3 className="g7-h4">Olası çakışmalar</h3>
                <ul className="g11-records">
                  {data.conflicts.map((c) => (
                    <li key={`${c.a}|${c.b}`} className="g11-conflict">
                      <span>{label(c.a)}</span>
                      <span className="g11-vs" aria-label="ile çelişiyor">↔</span>
                      <span>{label(c.b)}</span>
                      <span className="g7-note">{c.reason}</span>
                      <button
                        type="button"
                        className="btn small g11-keep"
                        disabled={busy === `${c.a}|${c.b}`}
                        onClick={() => void run(`${c.a}|${c.b}`, () => api.g11KeepBoth(c.a, c.b))}
                      >
                        İkisi de geçerli
                      </button>
                    </li>
                  ))}
                </ul>
                <p className="g7-note">Ajan sohbette hangisinin geçerli olduğunu soru kartıyla sorar; istersen geçersiz olanı aşağıdan sil veya ikisini de geçerli say.</p>
              </section>
            )}

            <LayerSection title="Oturum" desc="Yalnızca bu sohbet; bağlam sıkıştırılsa da kalır, sohbet kapanınca silinir." records={data.session} empty="Bu sohbette oturum notu yok." {...shared} />

            {project === null ? (
              <section className="g11-section">
                <h3 className="g7-h4">Proje</h3>
                <p className="g7-note">Açık bir klasör yok.</p>
              </section>
            ) : project.trusted ? (
              <LayerSection
                title="Proje"
                desc="Bu projedeki kararlar, kısıtlar, terimler ve notlar (.kor/knowledge.json)."
                records={project.records}
                empty={project.error ?? 'Bilgi tabanında kayıt yok. Ajan, netleşen kararları buraya kaydeder.'}
                {...shared}
              />
            ) : (
              <section className="g11-section">
                <h3 className="g7-h4">Proje</h3>
                <p className="g7-note">Bu klasör güvenilir olarak işaretlenmedi; proje hafızası yüklenmez ve yazılmaz.</p>
              </section>
            )}

            <LayerSection
              title="Global"
              desc="Tüm sohbetlerde ve projelerde geçerli (memory.md)."
              records={data.global}
              empty="Global hafıza boş."
              action={
                <button
                  type="button"
                  className="btn small"
                  onClick={() => {
                    closeG11Panel()
                    onOpenGlobalEditor()
                  }}
                >
                  <Pencil size={14} aria-hidden="true" /> Düzenle
                </button>
              }
              {...shared}
            />
          </>
        )}
      </div>
    </G7Modal>
  )
}
