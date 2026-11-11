// §27.3 dallar paneli (ADR 0063): oturumun dal ağacı (ana → dal → alt dal), iki dalın karşılaştırması,
// bir dalın sonucunu ana oturuma alma (dosyalar ReviewPanel ile tek tek), dalı unutma.
// CLI: kor sessions fork|tree|compare|merge|unfork.
import { ArrowLeft, Columns2, ExternalLink, GitMerge, RefreshCw, Trash2 } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { ForkComparison, MergePlan } from '../../../ipc-ek14.ts'
import { errorMessage } from '../../api.ts'
import { forkApi } from '../../ek14-api.ts'
import { ReviewPanel } from '../../review/ReviewPanel.tsx'
import type { KorPanelProps } from '../registry.ts'
import { acceptedFiles, flatten } from './merge.ts'

function Compare({ c, onBack }: { c: ForkComparison; onBack: () => void }) {
  return (
    <>
      <div className="kg-card-head">
        <button type="button" className="action-btn" onClick={onBack} aria-label="Dal listesine dön" title="Geri"><ArrowLeft size={16} aria-hidden="true" /></button>
        <h2 className="ks-title">Karşılaştırma</h2>
      </div>
      <p className="ks-label">Ortak geçmiş: {c.common} mesaj</p>
      {[c.a, c.b].map((side) => (
        <div key={side.id} className="ks-entry">
          <div className="ks-entry-head"><strong translate="no">{side.id}</strong><span className="ks-badge">{side.after} mesaj sonra</span></div>
          {side.conclusion && <p className="ks-entry-text" translate="no">{side.conclusion}</p>}
          {side.changed.length > 0 && <p className="ks-entry-meta muted" translate="no">Değişen: {side.changed.join(', ')}</p>}
        </div>
      ))}
      {c.files.length === 0 ? (
        <p className="ks-empty">İki dalın dosyaları aynı.</p>
      ) : (
        c.files.map((f) => (
          <details key={f.path} className="ks-more">
            <summary translate="no">{f.path} <span className="stat-add">+{f.added}</span> <span className="stat-del">−{f.removed}</span></summary>
            <pre className="ks-code" translate="no">{f.diff}</pre>
          </details>
        ))
      )}
    </>
  )
}

function Merge({ plan, onDone, onBack }: { plan: MergePlan; onDone: (msg: string) => void; onBack: () => void }) {
  const [note, setNote] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const apply = (files: { path: string; content: string | null }[]) =>
    forkApi
      .mergeApply(plan.fork, files, note)
      .then((r) => onDone(`${r.written.length} dosya yazıldı${r.note ? ', sonuç ana oturuma eklendi' : ''}. Geri almak için: kor undo ${plan.parent}`))
      .catch((e: unknown) => setError(errorMessage(e)))
  return (
    <>
      <div className="kg-card-head">
        <button type="button" className="action-btn" onClick={onBack} aria-label="Dal listesine dön" title="Geri"><ArrowLeft size={16} aria-hidden="true" /></button>
        <h2 className="ks-title">Ana oturuma al</h2>
      </div>
      <pre className="ks-code" translate="no">{plan.summary}</pre>
      <label className="ks-check"><input type="checkbox" checked={note} onChange={(e) => setNote(e.target.checked)} /> Sonuç notunu ana oturuma ekle</label>
      {plan.skipped.length > 0 && <p className="ks-warn">Atlanan (çok büyük ya da metin değil): <span translate="no">{plan.skipped.join(', ')}</span></p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      {plan.files.length > 0 ? (
        <div data-kor-cmd="fork.mergeApply">
          <ReviewPanel toolId={`fork-${plan.fork}`} files={plan.files} onAnswer={(approval, decisions) => void apply(acceptedFiles(plan.files, approval, decisions))} />
        </div>
      ) : (
        <div className="ks-actions">
          <p className="ks-label">Dalda dosya değişikliği yok.</p>
          <button type="button" className="btn primary" onClick={() => void apply([])} data-kor-cmd="fork.mergeApply"><GitMerge size={14} aria-hidden="true" /> Sonucu al</button>
        </div>
      )}
    </>
  )
}

export function ForksPanel({ sessionId }: KorPanelProps) {
  const [all, setAll] = useState(false)
  const scope = all ? null : sessionId
  const { data, error, mutate } = useSWR(['fork:tree', scope], () => forkApi.tree(scope), { revalidateOnFocus: false })
  const [picked, setPicked] = useState<string[]>([])
  const [view, setView] = useState<{ kind: 'compare'; c: ForkComparison } | { kind: 'merge'; plan: MergePlan } | null>(null)
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const fail = (e: unknown) => setNote({ text: errorMessage(e), ok: false })
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p.slice(-1), id]))

  if (view?.kind === 'compare') return <section className="ks"><Compare c={view.c} onBack={() => setView(null)} /></section>
  if (view?.kind === 'merge') {
    return (
      <section className="ks">
        <Merge plan={view.plan} onBack={() => setView(null)} onDone={(text) => { setView(null); setNote({ text, ok: true }); void mutate() }} />
      </section>
    )
  }
  const rows = flatten(data ?? [])
  return (
    <section className="ks" aria-label="Konuşma dalları">
      <div className="kg-card-head">
        <h2 className="ks-title">Dallar</h2>
        <label className="ks-check"><input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} /> Tüm oturumlar</label>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Dal ağacını yenile" title="Yenile" data-kor-cmd="fork.tree"><RefreshCw size={16} aria-hidden="true" /></button>
      </div>
      {note && <p className={note.ok ? 'ks-ok' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">Dallar okunuyor…</p>
      ) : rows.length === 0 ? (
        <div className="ks-empty">
          <p>{sessionId && !all ? 'Bu oturumun dalı yok.' : 'Henüz dallandırılmış oturum yok.'}</p>
          <p className="ks-note">Bir mesajın eylem çubuğundaki dal simgesiyle (Buradan dallan) o noktadan yeni bir oturum açılır; ana oturum değişmez.</p>
        </div>
      ) : (
        <ul className="ks-list" data-kor-cmd="fork.tree">
          {rows.map(({ node, depth }) => (
            <li key={node.id} className="ks-row" style={{ marginLeft: depth * 16 }}>
              <input type="checkbox" checked={picked.includes(node.id)} onChange={() => toggle(node.id)} aria-label={`${node.title || node.id} karşılaştırmaya seç`} disabled={node.missing} />
              <span className="kg-link-meta" translate="no">{node.missing ? '(silinmiş)' : node.title || node.id}</span>
              {node.at != null && <span className="ks-badge">@{node.at}</span>}
              {node.model && <span className="ks-chip" translate="no">{node.model}</span>}
              {node.merged != null && <span className="ks-badge on">birleştirildi</span>}
              <button type="button" className="action-btn" onClick={() => window.dispatchEvent(new CustomEvent('kor:fork-open', { detail: { id: node.id, worktree: node.worktree } }))} aria-label="Oturumu aç" title="Aç" disabled={node.missing}>
                <ExternalLink size={14} aria-hidden="true" />
              </button>
              {node.at != null && (
                <>
                  <button type="button" className="action-btn" onClick={() => forkApi.mergePlan(node.id).then((plan) => setView({ kind: 'merge', plan })).catch(fail)} aria-label="Sonucu ana oturuma al" title="Ana oturuma al" data-kor-cmd="fork.mergePlan">
                    <GitMerge size={14} aria-hidden="true" />
                  </button>
                  <button type="button" className="action-btn reject" onClick={() => forkApi.remove(node.id, true).then(() => mutate()).catch(fail)} aria-label="Dalı unut ve çalışma kopyasını kaldır" title="Dalı unut" data-kor-cmd="fork.remove">
                    <Trash2 size={14} aria-hidden="true" />
                  </button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      <div className="ks-actions">
        <button
          type="button"
          className="btn"
          disabled={picked.length !== 2}
          onClick={() => forkApi.compare(picked[0]!, picked[1]!).then((c) => setView({ kind: 'compare', c })).catch(fail)}
          data-kor-cmd="fork.compare"
        >
          <Columns2 size={14} aria-hidden="true" /> Seçili iki dalı karşılaştır
        </button>
      </div>
    </section>
  )
}
