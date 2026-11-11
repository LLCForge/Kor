// Toplu "Kor ile düzenle" (§27.6, ADR 0052): dosya yöneticisinden çoklu seçim (`osint:edit` olayı) → talimat →
// model işlem listesini önerir (`batch.plan`) → önizleme tablosu (eski → yeni / işlem) → onay → `batch.apply`
// (`kor undo` ile geri alınır). Silme işlemi yoktur; çakışan adlar numaralanır.
import { FilePlus2, Loader2, Undo2, Wand2, X } from 'lucide-react'
import { useEffect, useId, useMemo, useState } from 'react'
import type { BatchApplied, BatchPlan } from '../../ipc-batch.ts'
import { api, errorMessage } from '../api.ts'
import { Modal } from '../NameDialog.tsx'
import { batchApi } from './batch-bridge.ts'
import { commonDir, previewRows, shortPath } from './batch-shape.ts'
import '../panels/graph/panel.css'
import '../panels/security/panel.css'
import './korfolder.css'

type Phase =
  | { kind: 'input' }
  | { kind: 'planning' }
  | { kind: 'preview'; plan: BatchPlan }
  | { kind: 'applying'; plan: BatchPlan }
  | { kind: 'done'; result: BatchApplied }

const EXAMPLES = ['tarihi başa al, boşlukları tire yap', "hepsini webp'e çevir ve 1200 px yap", "PDF'leri birleştir", "EXIF'leri sil"]

export function BatchEditDialog({ root }: { root: string | null }) {
  const titleId = useId()
  const [files, setFiles] = useState<string[] | null>(null)
  const [instruction, setInstruction] = useState('')
  const [phase, setPhase] = useState<Phase>({ kind: 'input' })
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const onEdit = (e: Event) => {
      const detail = (e as CustomEvent<{ files?: unknown }>).detail
      const list = Array.isArray(detail?.files) ? detail.files.filter((f): f is string => typeof f === 'string') : []
      if (list.length === 0) return
      setFiles(list)
      setInstruction('')
      setPhase({ kind: 'input' })
      setError(null)
    }
    window.addEventListener('osint:edit', onEdit)
    return () => window.removeEventListener('osint:edit', onEdit)
  }, [])

  const base = useMemo(() => (files ? commonDir(files) ?? root : root), [files, root])

  if (!files) return null

  const close = () => {
    if (phase.kind === 'preview') void batchApi.cancel(phase.plan.id).catch(() => undefined)
    setFiles(null)
  }

  const addFiles = async () => {
    try {
      const more = await api.pickFiles()
      setFiles((prev) => [...new Set([...(prev ?? []), ...more])])
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  const plan = async () => {
    setError(null)
    setPhase({ kind: 'planning' })
    try {
      setPhase({ kind: 'preview', plan: await batchApi.plan(files, instruction.trim()) })
    } catch (err) {
      setError(errorMessage(err))
      setPhase({ kind: 'input' })
    }
  }

  const apply = async (p: BatchPlan) => {
    setError(null)
    setPhase({ kind: 'applying', plan: p })
    try {
      setPhase({ kind: 'done', result: await batchApi.apply(p.id) })
    } catch (err) {
      setError(errorMessage(err))
      setPhase({ kind: 'preview', plan: p })
    }
  }

  const rows = phase.kind === 'preview' || phase.kind === 'applying' ? previewRows(phase.plan.rows, base) : []

  return (
    <Modal onClose={close} labelledBy={titleId}>
      <div className="dialog-body kb-dialog">
        <div className="kb-head">
          <h2 id={titleId} className="dialog-title">
            <Wand2 size={16} aria-hidden="true" /> Kor ile düzenle
          </h2>
          <button type="button" className="action-btn" onClick={close} aria-label="Kapat" title="Kapat">
            <X size={16} aria-hidden="true" />
          </button>
        </div>

        <details className="kb-files">
          <summary>{files.length} dosya seçili</summary>
          <ul translate="no">
            {files.map((f) => (
              <li key={f}>{shortPath(f, base)}</li>
            ))}
          </ul>
        </details>

        {(phase.kind === 'input' || phase.kind === 'planning') && (
          <form
            className="kb-form"
            onSubmit={(e) => {
              e.preventDefault()
              if (instruction.trim()) void plan()
            }}
          >
            <label htmlFor={`${titleId}-i`} className="kb-label">Ne yapılsın?</label>
            <textarea
              id={`${titleId}-i`}
              className="kb-input"
              rows={3}
              maxLength={4096}
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              placeholder={EXAMPLES[0]}
              disabled={phase.kind === 'planning'}
            />
            <div className="kb-examples" aria-label="Örnekler">
              {EXAMPLES.map((x) => (
                <button key={x} type="button" className="ks-chip" onClick={() => setInstruction(x)} disabled={phase.kind === 'planning'}>
                  {x}
                </button>
              ))}
            </div>
            <p className="muted kb-note">Kor önce yapılacakları gösterir; onaylamadan hiçbir dosya değişmez. Silme yoktur, yalnızca yeniden adlandırma, taşıma ve yeni dosya.</p>
            <div className="dialog-actions">
              <button type="button" className="btn small" onClick={() => void addFiles()} disabled={phase.kind === 'planning'}>
                <FilePlus2 size={14} aria-hidden="true" /> Dosya ekle
              </button>
              <div className="dialog-actions-right">
                <button type="button" className="btn" onClick={close}>İptal</button>
                <button type="submit" className="btn primary" disabled={!instruction.trim() || phase.kind === 'planning'} data-kor-cmd="batch.plan">
                  {phase.kind === 'planning' ? <Loader2 size={14} className="spin" aria-hidden="true" /> : null}
                  {phase.kind === 'planning' ? 'Planlanıyor…' : 'Önizle'}
                </button>
              </div>
            </div>
          </form>
        )}

        {(phase.kind === 'preview' || phase.kind === 'applying') && (
          <>
            {rows.length === 0 ? (
              <p className="muted ks-empty" role="status">Bu talimatla yapılabilecek bir işlem bulunamadı. Talimatı değiştirip yeniden deneyin.</p>
            ) : (
              <div className="kb-table-wrap">
                <table className="kb-table">
                  <caption className="sr-only">Önizleme: eski ad, yeni ad ve işlem</caption>
                  <thead>
                    <tr>
                      <th scope="col">İşlem</th>
                      <th scope="col">Eski</th>
                      <th scope="col">Yeni</th>
                    </tr>
                  </thead>
                  <tbody translate="no">
                    {rows.map((r) => (
                      <tr key={r.key}>
                        <td translate="yes">
                          {r.op}
                          {r.inPlace && <span className="ks-chip kb-chip">yerinde</span>}
                        </td>
                        <td>{r.from}</td>
                        <td>
                          {r.to}
                          {r.detail && <span className="muted kb-detail"> {r.detail}</span>}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            {phase.plan.warnings.length > 0 && (
              <ul className="kb-warnings" aria-label="Uygulanmayacak öneriler" translate="no">
                {phase.plan.warnings.map((w) => (
                  <li key={w}>{w}</li>
                ))}
              </ul>
            )}
            <div className="dialog-actions">
              <button type="button" className="btn small" onClick={() => setPhase({ kind: 'input' })} disabled={phase.kind === 'applying'}>
                Talimatı değiştir
              </button>
              <div className="dialog-actions-right">
                <button type="button" className="btn" onClick={close} disabled={phase.kind === 'applying'}>İptal</button>
                <button
                  type="button"
                  className="btn primary"
                  onClick={() => void apply(phase.plan)}
                  disabled={rows.length === 0 || phase.kind === 'applying'}
                  data-kor-cmd="batch.apply"
                >
                  {phase.kind === 'applying' ? 'Uygulanıyor…' : `${rows.length} işlemi uygula`}
                </button>
              </div>
            </div>
          </>
        )}

        {phase.kind === 'done' && (
          <div className="kb-done" role="status">
            <p>{phase.result.applied} işlem uygulandı.</p>
            {phase.result.failed.length > 0 && (
              <ul className="kb-warnings" aria-label="Başarısız işlemler" translate="no">
                {phase.result.failed.map((f) => (
                  <li key={`${f.row.from}>${f.row.to}`}>
                    {shortPath(f.row.from, base)}: {f.error}
                  </li>
                ))}
              </ul>
            )}
            {phase.result.undo && (
              <p className="muted kb-note">
                <Undo2 size={14} aria-hidden="true" /> Geri almak için: <code translate="no">kor undo --session {phase.result.undo}</code>
              </p>
            )}
            <div className="dialog-actions">
              <span />
              <div className="dialog-actions-right">
                <button type="button" className="btn primary" onClick={close}>Tamam</button>
              </div>
            </div>
          </div>
        )}

        {error && <p className="error-text" role="alert">{error}</p>}
      </div>
    </Modal>
  )
}
