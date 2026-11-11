import { Check, ChevronDown, ChevronRight, Pencil, TriangleAlert, X } from 'lucide-react'
import { useId, useReducer, useState } from 'react'
import type { Approval } from '../../../core/agent.ts'
import { destructiveReason, type FileDecisions, type ReviewFile } from '../../ipc-review.ts'
import { lineStats } from '../chat-state.ts'
import { CodeEditor } from '../CodeEditor.tsx'
import { DiffView } from '../DiffView.tsx'
import { LangBadge } from '../LangBadge.tsx'
import { Modal } from '../NameDialog.tsx'
import { counts, initialChoices, reviewReducer, shownAfter, toAnswer, type Choice } from './review-state.ts'
import './panel.css'

interface ReviewPanelProps {
  toolId: string
  files: readonly ReviewFile[]
  destructive?: string
  onAnswer: (approval: Approval, files?: FileDecisions) => void
}

/** Yazmadan önce dosya bazında kabul / düzenle / reddet (§12.3); yıkıcı istek kırmızı işaretlenir. */
export function ReviewPanel({ toolId, files, destructive, onAnswer }: ReviewPanelProps) {
  const [choices, dispatch] = useReducer(reviewReducer, files, initialChoices)
  const [closed, setClosed] = useState<ReadonlySet<string>>(() => new Set(files.length > 3 ? files.map((f) => f.path) : []))
  const [editing, setEditing] = useState<ReviewFile | null>(null)
  const titleId = useId()
  const answer = toAnswer(files, choices)
  const n = counts(files, choices)
  const toggle = (p: string) => setClosed((prev) => { const s = new Set(prev); if (s.has(p)) s.delete(p); else s.add(p); return s })
  const set = (path: string, choice: Choice) => dispatch({ type: 'set', path, choice })

  return (
    <div className={`review${destructive ? ' danger' : ''}`}>
      {destructive && (
        <p className="review-danger" role="alert">
          <TriangleAlert size={14} aria-hidden="true" /> Yıkıcı işlem: {destructiveReason(destructive)}
        </p>
      )}
      <p className="approval-text">
        Ajan <strong>{files.length === 1 ? 'bir dosyayı' : `${files.length} dosyayı`}</strong> değiştirmek istiyor. Yazılmadan önce dosya dosya karar ver:
      </p>
      <ul className="review-files">
        {files.map((f) => {
          const choice = choices[f.path] ?? { kind: 'accept' }
          const after = shownAfter(f, choice)
          const open = !closed.has(f.path)
          const s = after === null ? null : lineStats(f.before, after)
          return (
            <li key={f.path} className={`review-file ${choice.kind}`}>
              <div className="review-file-head">
                <button type="button" className="action-btn" onClick={() => toggle(f.path)} aria-expanded={open} aria-label={open ? 'Diff’i daralt' : 'Diff’i genişlet'} title={open ? 'Daralt' : 'Genişlet'}>
                  {open ? <ChevronDown size={13} aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
                </button>
                <LangBadge path={f.path} />
                <span className="review-path" title={f.path} translate="no">{f.path}</span>
                {f.before === null && <span className="review-tag">yeni</span>}
                {after === null && <span className="review-tag del">silinecek</span>}
                {choice.kind === 'edit' && <span className="review-tag">düzenlendi</span>}
                {s && (s.added > 0 || s.removed > 0) && (
                  <span className="line-stats">
                    <span className="stat-add">+{s.added}</span>
                    <span className="stat-del">−{s.removed}</span>
                  </span>
                )}
                <span className="review-choice" role="group" aria-label={`${f.path} için karar`}>
                  <button type="button" className="action-btn" aria-pressed={choice.kind === 'accept'} onClick={() => set(f.path, { kind: 'accept' })} aria-label="Kabul et" title="Kabul et">
                    <Check size={14} aria-hidden="true" />
                  </button>
                  <button type="button" className="action-btn" aria-pressed={choice.kind === 'edit'} onClick={() => setEditing(f)} aria-label="Düzenle" title="Düzenleyerek kabul et">
                    <Pencil size={14} aria-hidden="true" />
                  </button>
                  <button type="button" className="action-btn reject" aria-pressed={choice.kind === 'reject'} onClick={() => set(f.path, { kind: 'reject' })} aria-label="Reddet" title="Reddet (yazılmaz)">
                    <X size={14} aria-hidden="true" />
                  </button>
                </span>
              </div>
              {open && choice.kind !== 'reject' && (
                <div className="review-diff">
                  <DiffView change={{ path: f.path, before: f.before, after: after ?? '' }} mode="unified" reviewable={false} />
                </div>
              )}
            </li>
          )
        })}
      </ul>
      <div className="approval-actions">
        <button
          type="button"
          className={`btn ${destructive ? 'danger' : 'primary approve'}`}
          data-kor-cmd="approve"
          onClick={() => onAnswer(answer.approval, answer.files)}
        >
          <Check size={14} aria-hidden="true" />{' '}
          {answer.approval === 'no' ? 'Hiçbirini yazma' : n.rejected + n.edited === 0 ? 'Tümünü yaz' : `Seçilenleri yaz (${n.accepted + n.edited}/${files.length})`}
        </button>
        {!destructive && !answer.files && answer.approval === 'yes' && (
          <button type="button" className="btn" data-kor-cmd="approve" onClick={() => onAnswer('always')}>
            Bu oturumda hep izin ver
          </button>
        )}
        {files.length > 1 && (
          <>
            <button type="button" className="btn ghost" onClick={() => dispatch({ type: 'all', kind: 'accept' })}>Tümünü kabul et</button>
            <button type="button" className="btn ghost" onClick={() => dispatch({ type: 'all', kind: 'reject' })}>Tümünü reddet</button>
          </>
        )}
        <button type="button" className="btn ghost deny" data-kor-cmd="approve" onClick={() => onAnswer('no')}>
          <X size={14} aria-hidden="true" /> Reddet
        </button>
      </div>
      {editing && (
        <Modal onClose={() => setEditing(null)} labelledBy={titleId}>
          <div className="dialog-body review-editor">
            <h2 id={titleId} className="dialog-title" translate="no">{editing.path}</h2>
            <p className="dialog-desc">Kaydettiğin içerik ajanın önerisi yerine yazılır.</p>
            <CodeEditor
              path={`kor-review/${toolId}/${editing.path}`}
              content={shownAfter(editing, choices[editing.path]) ?? editing.before ?? ''}
              mtimeMs={0}
              line={null}
              lineSeq={0}
              onRefresh={() => {}}
              onClose={() => setEditing(null)}
              write={async (content) => {
                set(editing.path, { kind: 'edit', content })
                return { ok: true, mtimeMs: 0 }
              }}
            />
          </div>
        </Modal>
      )}
    </div>
  )
}
