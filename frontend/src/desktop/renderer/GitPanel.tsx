// cdesktop Git kartı: depo adı, dal rozeti, ↑ileri/↓geri, ⋯, "Pull request aç ⌄" (ya da "PR #7 ↗"),
// "Çalışma dalı" kartı; PR diyaloğu Başlık / Açıklama / Taban dal / Taslak. gh CLI varsa PR gerçekten
// oluşturulur, yoksa tarayıcıda GitHub compare sayfası önceden doldurulmuş açılır.
import { AlertTriangle, ArrowDown, ArrowUp, ChevronDown, ExternalLink, GitBranch, GitMerge, GitPullRequest, RefreshCw, Upload, Wrench, X } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { CreatePrInput, GitStatus } from '../ipc-git.ts'
import { api, errorMessage } from './api.ts'
import { Modal } from './NameDialog.tsx'
import { GitFlowSection } from './g5/GitFlowSection.tsx'
import { ForgeSection } from './forge/ForgeSection.tsx'
import { usePopover } from './use-popover.ts'

const GIT_KEY = 'git:status'

function CreatePrDialog({ status, onClose, onDone }: { status: GitStatus; onClose: () => void; onDone: (url: string, created: boolean) => void }) {
  const titleId = useId()
  const fid = useId()
  const { data: branches } = useSWR('git:branches', () => api.git.branches(), { revalidateOnFocus: false })
  const [title, setTitle] = useState(status.branch ?? '')
  const [body, setBody] = useState('')
  const [base, setBase] = useState('')
  const [draft, setDraft] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!base && branches?.defaultBase) setBase(branches.defaultBase)
  }, [branches, base])
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const input: CreatePrInput = { title: title.trim(), body, base: base.trim(), draft }
      const res = await api.git.createPr(input)
      onDone(res.url, res.created)
      onClose()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const bases = (branches?.local ?? []).filter((b) => b !== status.branch)
  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <form className="dialog-body cdx-pr-form" onSubmit={(e) => void submit(e)}>
        <h2 id={titleId} className="dialog-title">Pull request oluştur</h2>
        <p className="dialog-desc">
          <code>{status.branch}</code> dalı için pull request aç.{!status.hasGh && ' gh CLI bulunamadı: form tarayıcıda GitHub üzerinde açılır.'}
        </p>
        <label htmlFor={`${fid}-t`}>Başlık</label>
        <input id={`${fid}-t`} className="text-input" value={title} onChange={(e) => setTitle(e.target.value)} maxLength={256} autoFocus />
        <label htmlFor={`${fid}-b`}>Açıklama (isteğe bağlı)</label>
        <textarea id={`${fid}-b`} className="text-input dialog-textarea" rows={5} value={body} onChange={(e) => setBody(e.target.value)} />
        <label htmlFor={`${fid}-base`}>Taban dal</label>
        {bases.length > 0 ? (
          <select id={`${fid}-base`} className="g7-select" value={base} onChange={(e) => setBase(e.target.value)}>
            {!bases.includes(base) && base && <option value={base}>{base}</option>}
            {bases.map((b) => (
              <option key={b} value={b}>{b}</option>
            ))}
          </select>
        ) : (
          <input id={`${fid}-base`} className="text-input" value={base} onChange={(e) => setBase(e.target.value)} placeholder="main" />
        )}
        <label className="g9-check">
          <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} /> Taslak olarak oluştur
        </label>
        {error && <p className="error-text" role="alert">{error}</p>}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose} disabled={busy}>İptal</button>
            <button type="submit" className="btn primary" disabled={busy || !title.trim() || !base.trim()}>
              {busy ? 'Oluşturuluyor…' : 'PR oluştur'}
            </button>
          </div>
        </div>
      </form>
    </Modal>
  )
}

export function GitPanel() {
  const { data, error, mutate, isLoading } = useSWR(GIT_KEY, () => api.git.status(), { revalidateOnFocus: true, dedupingInterval: 5_000 })
  const [busy, setBusy] = useState<'push' | 'merge' | 'rebase' | 'abort' | 'continue' | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [prOpen, setPrOpen] = useState(false)
  const menu = usePopover<HTMLDivElement>()
  useEffect(() => {
    const onPr = () => setPrOpen(true)
    window.addEventListener('cdx:git:pr', onPr)
    return () => window.removeEventListener('cdx:git:pr', onPr)
  }, [])

  const push = async () => {
    setBusy('push')
    setMessage(null)
    try {
      await api.git.push()
      setMessage('Dal origin’e itildi.')
      await mutate()
    } catch (err) {
      setMessage(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const gitOp = async (kind: 'merge' | 'rebase' | 'abort' | 'continue') => {
    if (!data) return
    setBusy(kind)
    setMessage(null)
    try {
      const base = data.defaultBase ?? 'main'
      const out =
        kind === 'merge' ? await api.git.merge(base) : kind === 'rebase' ? await api.git.rebase(base) : kind === 'abort' ? await api.git.abortOperation() : await api.git.continueOperation()
      setMessage(out.trim().split('\n').at(-1) || (kind === 'abort' ? 'İşlem iptal edildi.' : 'Tamamlandı.'))
      await mutate()
    } catch (err) {
      setMessage(errorMessage(err))
      await mutate()
    } finally {
      setBusy(null)
    }
  }
  const resolveWithAgent = () => {
    if (!data) return
    const list = data.conflicts.map((f) => `- ${f}`).join('\n')
    window.dispatchEvent(new CustomEvent('cdx:draft', { detail: `Süren ${data.operation === 'rebase' ? 'rebase' : 'merge'} işleminde çakışma var. Şu dosyalardaki çakışma işaretlerini (<<<<<<<, =======, >>>>>>>) iki tarafın amacını koruyarak çöz, dosyaları kaydet ve çözümü kısaca açıkla:\n${list}\n\nÇözüm talimatları: (varsa buraya yaz)` }))
  }

  if (error) return <p className="error-text pad">{errorMessage(error)}</p>
  if (!data || isLoading) return <p className="muted pad">Git durumu okunuyor…</p>
  if (!data.isRepo) return <p className="muted pad">Bu klasör bir git deposu değil.</p>

  return (
    <div className="cdx-git">
      {data.operation && (
        <section className="cdx-git-banner" role="alert">
          <p className="cdx-git-banner-title">
            <AlertTriangle size={14} aria-hidden="true" /> {data.operation === 'rebase' ? 'Rebase sürüyor' : 'Merge sürüyor'}
            {data.branch && data.defaultBase && <span className="muted"> · {data.branch} → {data.defaultBase}</span>}
          </p>
          {data.conflicts.length > 0 ? (
            <>
              <p className="g9-small">Çakışmalı dosyalar ({data.conflicts.length}):</p>
              <ul className="cdx-git-conflicts">{data.conflicts.map((f) => <li key={f}><code>{f}</code></li>)}</ul>
            </>
          ) : (
            <p className="g9-small muted">Çakışma kalmadı; devam edebilirsin.</p>
          )}
          <div className="cdx-git-actions">
            {data.conflicts.length > 0 ? (
              <button type="button" className="btn small cdx-btn-accent" onClick={resolveWithAgent}><Wrench size={13} aria-hidden="true" /> Çakışmaları çöz (ajan)</button>
            ) : (
              <button type="button" className="btn small cdx-btn-accent" onClick={() => void gitOp('continue')} disabled={busy !== null}>Devam et</button>
            )}
            <button type="button" className="btn small" onClick={() => void api.g4.openInEditor('.', null, 'system')}>Editörde aç</button>
            <button type="button" className="btn small cdx-btn-danger" onClick={() => void gitOp('abort')} disabled={busy !== null}><X size={13} aria-hidden="true" /> {data.operation === 'rebase' ? 'Rebase’i iptal et' : 'Merge’i iptal et'}</button>
          </div>
        </section>
      )}
      <section className="cdx-git-card">
        <div className="cdx-git-repo">{data.repoName}</div>
        <div className="cdx-git-row">
          <span className="cdx-branch-pill" title={data.branch ?? 'ayrık HEAD'}>
            <GitBranch size={13} aria-hidden="true" />
            <span className="truncate">{data.branch ?? 'HEAD'}</span>
          </span>
          <span className="cdx-git-counts" aria-label={`ileri ${data.ahead ?? 0}, geri ${data.behind ?? 0}`}>
            {data.ahead !== null && data.ahead > 0 && (
              <span className="stat-add"><ArrowUp size={12} aria-hidden="true" />{data.ahead}</span>
            )}
            {data.behind !== null && data.behind > 0 && (
              <span className="stat-del"><ArrowDown size={12} aria-hidden="true" />{data.behind}</span>
            )}
            {data.dirty > 0 && <span className="muted" title="kaydedilmemiş değişiklik">●{data.dirty}</span>}
          </span>
          <div className="popover-anchor" ref={menu.ref}>
            <button type="button" className="action-btn" onClick={() => menu.setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={menu.open} aria-label="Git işlemleri" title="Git işlemleri">
              ⋯
            </button>
            {menu.open && (
              <div className="menu menu-left pop-in" role="menu">
                <button type="button" role="menuitem" className="menu-row" onClick={() => { menu.setOpen(false); void mutate() }}>
                  <RefreshCw size={14} aria-hidden="true" /> Yenile
                </button>
                <button type="button" role="menuitem" className="menu-row" onClick={() => { menu.setOpen(false); void push() }} disabled={busy !== null || !data.branch}>
                  <Upload size={14} aria-hidden="true" /> Push
                </button>
                {data.webUrl && (
                  <button type="button" role="menuitem" className="menu-row" onClick={() => { menu.setOpen(false); void api.git.openPr(data.webUrl!) }}>
                    <ExternalLink size={14} aria-hidden="true" /> GitHub’da aç
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
        <div className="cdx-git-actions">
          <button type="button" className="btn small cdx-btn-merge" onClick={() => void gitOp('merge')} disabled={busy !== null || !data.branch || !!data.operation} title={`${data.defaultBase ?? 'main'} dalını bu dala birleştir`}>
            <GitMerge size={13} aria-hidden="true" /> Merge
          </button>
          <button type="button" className="btn small cdx-btn-rebase" onClick={() => void gitOp('rebase')} disabled={busy !== null || !data.branch || !!data.operation} title={`${data.defaultBase ?? 'main'} üzerine rebase`}>
            <RefreshCw size={13} aria-hidden="true" /> Rebase
          </button>
          {data.pr ? (
            <button type="button" className="btn small cdx-pr-btn" onClick={() => void api.git.openPr(data.pr!.url)} title={data.pr.url}>
              <GitPullRequest size={14} aria-hidden="true" /> PR #{data.pr.number} <ExternalLink size={12} aria-hidden="true" />
            </button>
          ) : (
            <span className="cdx-split">
              <button type="button" className="btn small" onClick={() => setPrOpen(true)} disabled={!data.branch}>
                <GitPullRequest size={14} aria-hidden="true" /> Pull request aç
              </button>
              <button type="button" className="btn small cdx-split-more" onClick={() => void push()} disabled={busy !== null || !data.branch} title="Yalnızca push" aria-label="Yalnızca push">
                {busy === 'push' ? <RefreshCw size={13} className="spin" aria-hidden="true" /> : <ChevronDown size={13} aria-hidden="true" />}
              </button>
            </span>
          )}
        </div>
        {message && <p className="muted g9-small" role="status">{message}</p>}
      </section>

      <section className="cdx-git-card">
        <div className="cdx-git-subtitle"><GitBranch size={13} aria-hidden="true" /> Çalışma dalı</div>
        <div className="cdx-branch-box" title={data.branch ?? ''}>
          <span className="truncate">{data.branch ?? '—'}</span>
        </div>
        {data.remoteUrl && <p className="g7-mono g9-url">{data.remoteUrl}</p>}
      </section>

      <GitFlowSection onCommitted={() => void mutate()} />

      <ForgeSection />

      {prOpen && (
        <CreatePrDialog
          status={data}
          onClose={() => setPrOpen(false)}
          onDone={(url, created) => {
            setMessage(created ? `PR oluşturuldu: ${url}` : 'PR formu tarayıcıda açıldı.')
            void mutate()
          }}
        />
      )}
    </div>
  )
}
