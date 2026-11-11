// Git paneli: `kor commit` (hazırlanmış diff'ten Conventional Commits mesajı, düzenlenebilir, onayla commit) ve
// `kor changelog` (Keep a Changelog bölümü, istenirse CHANGELOG.md'ye yazılır). Rust: ns/gitflow.rs.
import { FileText, GitCommitHorizontal, Sparkles } from 'lucide-react'
import { useId, useState } from 'react'
import type { CommitProposal } from '../../ipc-shell.ts'
import { errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { CopyButton } from '../CopyButton.tsx'
import { gitflowApi } from '../panels/security/bridge.ts'
import '../panels/security/panel.css'

function CommitCard({ onCommitted }: { onCommitted: () => void }) {
  const id = useId()
  const [proposal, setProposal] = useState<CommitProposal | null>(null)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState<'gen' | 'commit' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)

  const generate = async (split: boolean) => {
    setBusy('gen')
    setError(null)
    setDone(null)
    try {
      const p = await gitflowApi.commitMessage(split)
      setProposal(p)
      setMessage(p.message)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const commit = async () => {
    const ok = await confirmAction({ title: 'Commit oluşturulsun mu?', message: `Hazırlanmış (staged) ${proposal?.files.length ?? 0} dosya bu mesajla commit edilecek.`, confirmLabel: 'Commit et' })
    if (!ok) return
    setBusy('commit')
    setError(null)
    try {
      setDone((await gitflowApi.commit(message)).summary)
      setProposal(null)
      setMessage('')
      onCommitted()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="cdx-git-card k5-card" aria-labelledby={`${id}-t`}>
      <div id={`${id}-t`} className="cdx-git-subtitle"><GitCommitHorizontal size={13} aria-hidden="true" /> Commit mesajı</div>
      <div className="ks-actions">
        <button type="button" className="btn small" onClick={() => void generate(false)} disabled={busy !== null} data-kor-cmd="gitflow.commitMessage">
          <Sparkles size={13} aria-hidden="true" />
          {busy === 'gen' ? 'Üretiliyor…' : 'Hazırlanmış diff’ten üret'}
        </button>
        <button type="button" className="btn small" onClick={() => void generate(true)} disabled={busy !== null} data-kor-cmd="gitflow.commitMessage" title="Mantıksal parçalara bölünmüş çoklu commit önerisi">
          Parçalara böl
        </button>
      </div>
      {proposal && (
        <div className="ks-form">
          <p className="muted g9-small">
            {`${proposal.files.length} dosya`}
            {proposal.scope && (
              <>
                {' · scope: '}
                <code translate="no">{proposal.scope}</code>
              </>
            )}
          </p>
          {proposal.breaking && <p className="ks-warn">Kırıcı değişiklik (BREAKING CHANGE) olarak işaretlendi.</p>}
          <label htmlFor={`${id}-m`} className="sr-only">Commit mesajı</label>
          <textarea id={`${id}-m`} className="text-input ks-textarea" value={message} onChange={(e) => setMessage(e.target.value)} maxLength={20000} spellCheck={false} />
          {proposal.parts.length > 1 && (
            <details>
              <summary>{`Önerilen parçalar (${proposal.parts.length})`}</summary>
              <ol className="k5-parts">
                {proposal.parts.map((p, i) => (
                  <li key={i}>
                    <p className="muted g9-small" translate="no">{p.files.join(', ')}</p>
                    <pre className="ks-code" translate="no">{p.message}</pre>
                    <button type="button" className="btn small" onClick={() => setMessage(p.message)}>Bu mesajı kullan</button>
                  </li>
                ))}
              </ol>
            </details>
          )}
          <div className="ks-actions">
            <button type="button" className="btn small primary" onClick={() => void commit()} disabled={busy !== null || !message.trim()} data-kor-cmd="gitflow.commit">
              {busy === 'commit' ? 'Commit ediliyor…' : 'Commit et…'}
            </button>
            <CopyButton getText={() => message} label="Mesajı kopyala" className="btn small" />
          </div>
        </div>
      )}
      {done && <p className="muted g9-small" role="status" translate="no">{done}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function ChangelogCard() {
  const id = useId()
  const [since, setSince] = useState('')
  const [result, setResult] = useState<{ section: string; since: string | null; count: number; path?: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async (write: boolean) => {
    if (write) {
      const ok = await confirmAction({ title: 'CHANGELOG.md güncellensin mi?', message: 'Bölüm, deponun kökündeki CHANGELOG.md dosyasına “Unreleased” başlığıyla yazılır (dosya yoksa oluşturulur).', confirmLabel: 'Yaz' })
      if (!ok) return
    }
    setBusy(true)
    setError(null)
    try {
      setResult(await gitflowApi.changelog(since.trim() || null, write))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="cdx-git-card k5-card" aria-labelledby={`${id}-t`}>
      <div id={`${id}-t`} className="cdx-git-subtitle"><FileText size={13} aria-hidden="true" /> Changelog</div>
      <div className="ks-inline">
        <label htmlFor={`${id}-s`} className="sr-only">Başlangıç etiketi ya da commit</label>
        <input id={`${id}-s`} className="text-input" value={since} onChange={(e) => setSince(e.target.value)} placeholder="Son etiketten beri (ör. v1.2.0)" maxLength={200} spellCheck={false} translate="no" />
        <button type="button" className="btn small" onClick={() => void run(false)} disabled={busy} data-kor-cmd="gitflow.changelog">Önizle</button>
      </div>
      {result && (
        <>
          <p className="muted g9-small">{result.since ? `${result.since} sonrası ${result.count} commit` : `${result.count} commit`}</p>
          {result.count === 0 ? (
            <p className="muted g9-small">Bu aralıkta commit yok.</p>
          ) : (
            <>
              <pre className="ks-code" translate="no">{result.section}</pre>
              <div className="ks-actions">
                <button type="button" className="btn small primary" onClick={() => void run(true)} disabled={busy} data-kor-cmd="gitflow.changelog">CHANGELOG.md’ye yaz…</button>
                <CopyButton getText={() => result.section} label="Kopyala" className="btn small" />
              </div>
            </>
          )}
          {result.path && <p className="muted g9-small" role="status" translate="no">{result.path}</p>}
        </>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

export function GitFlowSection({ onCommitted }: { onCommitted: () => void }) {
  return (
    <>
      <CommitCard onCommitted={onCommitted} />
      <ChangelogCard />
    </>
  )
}
