// Git paneli: Evrensel Git (§8.8, ADR 0040). Forge algılama, açıklaması yazılmış PR açma (`kor pr create`),
// açık PR'ler (`kor pr list|status`), persona ile inceleme (`kor review`, `kor pr review`) ve inceleme
// yorumlarına yanıt (`kor pr respond`: dosya başına kabul/düzenle/reddet, ADR 0030). Rust: ns/forge.rs.
import { Check, ExternalLink, GitPullRequest, MessageSquareReply, RefreshCw, ScanSearch, Send, Sparkles, X } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import { findingLocation, severityCounts, type PrDescription, type PrInfo, type RespondDraft, type ReviewResult } from '../../ipc-forge.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { CopyButton } from '../CopyButton.tsx'
import { DiffView } from '../DiffView.tsx'
import { forgeApi } from './bridge.ts'
import '../panels/security/panel.css'
import './forge.css'

const KIND_LABEL: Record<string, string> = { github: 'GitHub', gitlab: 'GitLab', gitea: 'Gitea/Forgejo', bitbucket: 'Bitbucket' }
const SEVERITY_LABEL: Record<string, string> = { critical: 'kritik', high: 'yüksek', medium: 'orta', low: 'düşük', info: 'bilgi' }

const openUrl = (url: string) => void api.git.openPr(url)

function ReviewResults({ result, target, onPosted }: { result: ReviewResult; target: string | number | null; onPosted?: (msg: string) => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { report } = result
  const post = async () => {
    if (target == null) return
    const ok = await confirmAction({ title: 'İnceleme gönderilsin mi?', message: `${report.findings.length} bulgu #${target} numaralı pull request'e yorum olarak yazılacak (satırı olanlar satır içi). Bu herkese görünür.`, confirmLabel: 'Gönder' })
    if (!ok) return
    setBusy(true)
    setError(null)
    try {
      const r = await forgeApi.prReviewPost(target, report)
      onPosted?.(`İnceleme gönderildi: ${r.inline} satır içi yorum${r.folded > 0 ? `, ${r.folded} yorum özete eklendi` : ''}.`)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="ks-form k6-review">
      <p className="muted g9-small">
        {`Persona: ${report.persona}`}
        {severityCounts(report.findings).map(([s, n]) => (
          <span key={s} className={`k6-sev k6-sev-${s}`}>{`${SEVERITY_LABEL[s]} ${n}`}</span>
        ))}
      </p>
      {report.findings.length === 0 ? (
        <p className="muted g9-small">Sorun bulunmadı.</p>
      ) : (
        <ul className="k6-findings">
          {report.findings.map((f, i) => (
            <li key={i} className={`k6-finding k6-sev-${f.severity}`}>
              <div className="k6-finding-head">
                <span className={`k6-sev k6-sev-${f.severity}`}>{SEVERITY_LABEL[f.severity]}</span>
                <code translate="no">{findingLocation(f)}</code>
                <span className="muted g9-small" translate="no">{f.rule}</span>
                {f.source === 'check' && <span className="muted g9-small">otomatik denetim</span>}
              </div>
              <p className="g9-small" translate="no">{f.message}</p>
            </li>
          ))}
        </ul>
      )}
      {report.model_error && <p className="ks-warn">{`Model incelemesi atlandı: ${report.model_error}`}</p>}
      <div className="ks-actions">
        {target != null && (
          <button type="button" className="btn small primary" onClick={() => void post()} disabled={busy} data-kor-cmd="forge.prReviewPost">
            <Send size={13} aria-hidden="true" /> {busy ? 'Gönderiliyor…' : 'PR’ye gönder…'}
          </button>
        )}
        <CopyButton getText={() => result.markdown} label="Markdown kopyala" className="btn small" />
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

function CreateCard({ onDone }: { onDone: () => void }) {
  const id = useId()
  const [desc, setDesc] = useState<PrDescription | null>(null)
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [base, setBase] = useState('')
  const [draft, setDraft] = useState(false)
  const [busy, setBusy] = useState<'gen' | 'create' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<{ url: string; number: number } | null>(null)

  const generate = async () => {
    setBusy('gen')
    setError(null)
    setDone(null)
    try {
      const d = await forgeApi.prDescribe(base.trim() || null)
      setDesc(d)
      setTitle(d.title)
      setBody(d.body)
      setBase(d.base)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }
  const create = async () => {
    if (!desc) return
    const ok = await confirmAction({
      title: 'Pull request açılsın mı?',
      message: desc.firstPush
        ? `${desc.head} dalı hiç gönderilmedi. Uzak depoya gönderilecek ve ${base} dalına pull request açılacak.`
        : `${desc.head} dalı gönderilecek ve ${base} dalına pull request açılacak.`,
      confirmLabel: 'Gönder ve aç',
    })
    if (!ok) return
    setBusy('create')
    setError(null)
    try {
      const r = await forgeApi.prCreate({ title: title.trim(), body, base: base.trim(), draft })
      setDone(r)
      setDesc(null)
      onDone()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }
  return (
    <div className="ks-form">
      <div className="ks-inline">
        <label htmlFor={`${id}-base`} className="sr-only">Taban dal</label>
        <input id={`${id}-base`} className="text-input" value={base} onChange={(e) => setBase(e.target.value)} placeholder="Taban dal (varsayılan dal)" maxLength={200} spellCheck={false} translate="no" />
        <button type="button" className="btn small" onClick={() => void generate()} disabled={busy !== null} data-kor-cmd="forge.prDescribe">
          <Sparkles size={13} aria-hidden="true" /> {busy === 'gen' ? 'Yazılıyor…' : 'Açıklama yaz'}
        </button>
      </div>
      {desc && (
        <>
          <p className="muted g9-small" translate="no">{`${desc.head} → ${base}`}</p>
          {desc.note && <p className="ks-warn">{desc.note}</p>}
          <label htmlFor={`${id}-t`}>Başlık</label>
          <input id={`${id}-t`} className="text-input" value={title} onChange={(e) => setTitle(e.target.value)} maxLength={256} translate="no" />
          <label htmlFor={`${id}-b`}>Açıklama</label>
          <textarea id={`${id}-b`} className="text-input ks-textarea" value={body} onChange={(e) => setBody(e.target.value)} maxLength={60000} spellCheck={false} translate="no" />
          <label className="g9-check">
            <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} /> Taslak olarak aç
          </label>
          <div className="ks-actions">
            <button type="button" className="btn small primary" onClick={() => void create()} disabled={busy !== null || !title.trim() || !base.trim()} data-kor-cmd="forge.prCreate">
              <GitPullRequest size={13} aria-hidden="true" /> {busy === 'create' ? 'Açılıyor…' : 'Gönder ve PR aç…'}
            </button>
          </div>
        </>
      )}
      {done && (
        <button type="button" className="btn small" onClick={() => openUrl(done.url)} title={done.url}>
          <ExternalLink size={13} aria-hidden="true" /> {`PR #${done.number} açıldı`}
        </button>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

function RespondView({ target, onClose }: { target: number; onClose: () => void }) {
  const [draft, setDraft] = useState<RespondDraft | null>(null)
  const [decisions, setDecisions] = useState<Record<string, 'accept' | 'reject' | 'edit'>>({})
  const [edited, setEdited] = useState<Record<string, string>>({})
  const [replies, setReplies] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState<'draft' | 'apply' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)

  const load = async () => {
    setBusy('draft')
    setError(null)
    try {
      const d = await forgeApi.prRespond(target)
      setDraft(d)
      setDecisions(Object.fromEntries(d.edits.map((e) => [e.path, 'accept'])))
      setEdited(Object.fromEntries(d.edits.map((e) => [e.path, e.after ?? ''])))
      setReplies(Object.fromEntries(d.replies.map((r) => [r.id, r.body])))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }
  const apply = async () => {
    if (!draft) return
    const files = draft.edits.filter((e) => decisions[e.path] !== 'reject').map((e) => ({ path: e.path, content: decisions[e.path] === 'edit' ? edited[e.path] ?? '' : e.after ?? '' }))
    const rs = Object.entries(replies).filter(([, b]) => b.trim()).map(([id, body]) => ({ id, body }))
    const ok = await confirmAction({ title: 'Değişiklikler gönderilsin mi?', message: `${files.length} dosya yazılıp commit edilecek ve gönderilecek; #${target} için ${rs.length} yanıt yazılacak.`, confirmLabel: 'Uygula ve gönder' })
    if (!ok) return
    setBusy('apply')
    setError(null)
    try {
      const r = await forgeApi.prRespondApply(target, files, rs)
      setDone(`${r.commit ? `${r.commit} · ` : ''}${r.sent} yanıt gönderildi.`)
      if (r.errors.length) setError(r.errors.join('\n'))
      setDraft(null)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }
  const comment = (id: string) => draft?.comments.find((c) => c.id === id)

  return (
    <div className="ks-form k6-respond">
      <div className="k6-finding-head">
        <strong>{`#${target} yorumlarına yanıt`}</strong>
        <button type="button" className="action-btn" onClick={onClose} aria-label="Kapat" title="Kapat"><X size={14} aria-hidden="true" /></button>
      </div>
      {!draft && !done && (
        <button type="button" className="btn small" onClick={() => void load()} disabled={busy !== null} data-kor-cmd="forge.prRespond">
          <Sparkles size={13} aria-hidden="true" /> {busy === 'draft' ? 'Taslak hazırlanıyor…' : 'Yanıt ve düzeltme taslağı hazırla'}
        </button>
      )}
      {draft && draft.comments.length === 0 && <p className="muted g9-small">Yanıtlanacak inceleme yorumu yok.</p>}
      {draft && draft.edits.map((e) => (
        <div key={e.path} className="k6-edit">
          <div className="k6-finding-head">
            <code translate="no">{e.path}</code>
            <div className="ks-actions" role="group" aria-label={`${e.path} kararı`}>
              {(['accept', 'edit', 'reject'] as const).map((d) => (
                <button key={d} type="button" className={`btn small${decisions[e.path] === d ? ' primary' : ''}`} aria-pressed={decisions[e.path] === d} onClick={() => setDecisions({ ...decisions, [e.path]: d })}>
                  {d === 'accept' ? 'Kabul' : d === 'edit' ? 'Düzenle' : 'Reddet'}
                </button>
              ))}
            </div>
          </div>
          {decisions[e.path] === 'edit' ? (
            <textarea className="text-input ks-textarea" aria-label={`${e.path} içeriği`} value={edited[e.path] ?? ''} onChange={(ev) => setEdited({ ...edited, [e.path]: ev.target.value })} spellCheck={false} translate="no" />
          ) : (
            <DiffView change={{ path: e.path, before: e.before, after: e.after ?? '' }} mode="unified" reviewable={false} />
          )}
        </div>
      ))}
      {draft && draft.replies.map((r) => (
        <div key={r.id} className="k6-reply">
          <blockquote className="muted g9-small" translate="no">{`${comment(r.id)?.author ?? ''}: ${comment(r.id)?.body ?? ''}`}</blockquote>
          <label className="sr-only" htmlFor={`k6-r-${r.id}`}>Yanıt</label>
          <textarea id={`k6-r-${r.id}`} className="text-input" rows={3} value={replies[r.id] ?? ''} onChange={(ev) => setReplies({ ...replies, [r.id]: ev.target.value })} translate="no" />
        </div>
      ))}
      {draft && draft.comments.length > 0 && (
        <div className="ks-actions">
          <button type="button" className="btn small primary" onClick={() => void apply()} disabled={busy !== null} data-kor-cmd="forge.prRespondApply">
            <MessageSquareReply size={13} aria-hidden="true" /> {busy === 'apply' ? 'Gönderiliyor…' : 'Uygula, gönder ve yanıtla…'}
          </button>
        </div>
      )}
      {done && <p className="muted g9-small" role="status" translate="no">{done}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

function PrListCard({ refreshKey }: { refreshKey: number }) {
  const { data, error, isLoading, mutate } = useSWR(['forge:prs', refreshKey], () => forgeApi.prList(), { revalidateOnFocus: false })
  const [review, setReview] = useState<{ pr: PrInfo; result: ReviewResult | null; busy: boolean; error: string | null } | null>(null)
  const [respond, setRespond] = useState<number | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const startReview = async (pr: PrInfo) => {
    setRespond(null)
    setReview({ pr, result: null, busy: true, error: null })
    try {
      setReview({ pr, result: await forgeApi.prReview(pr.number), busy: false, error: null })
    } catch (err) {
      setReview({ pr, result: null, busy: false, error: errorMessage(err) })
    }
  }
  return (
    <div className="ks-form" data-kor-cmd="forge.prList">
      <div className="k6-finding-head">
        <span className="muted g9-small">Açık pull request'ler</span>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Yenile" title="Yenile"><RefreshCw size={13} aria-hidden="true" /></button>
      </div>
      {isLoading && <p className="muted g9-small">Okunuyor…</p>}
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {data && data.prs.length === 0 && <p className="muted g9-small">Açık pull request yok.</p>}
      {data && data.prs.length > 0 && (
        <ul className="k6-prs">
          {data.prs.map((p) => (
            <li key={p.number}>
              <button type="button" className="link-btn" onClick={() => openUrl(p.url)} title={p.url} translate="no">{`#${p.number} ${p.title}`}</button>
              <span className="muted g9-small" translate="no">{`${p.head} → ${p.base} · @${p.author}${p.draft ? ' · taslak' : ''}`}</span>
              <span className="ks-actions">
                <button type="button" className="btn small" onClick={() => void startReview(p)} data-kor-cmd="forge.prReview"><ScanSearch size={13} aria-hidden="true" /> İncele</button>
                <button type="button" className="btn small" onClick={() => { setReview(null); setRespond(p.number) }} data-kor-cmd="forge.prRespond"><MessageSquareReply size={13} aria-hidden="true" /> Yanıtla</button>
              </span>
            </li>
          ))}
        </ul>
      )}
      {review?.busy && <p className="muted g9-small">{`#${review.pr.number} inceleniyor…`}</p>}
      {review?.error && <p className="error-text" role="alert">{review.error}</p>}
      {review?.result && <ReviewResults result={review.result} target={review.pr.number} onPosted={(m) => { setMessage(m); setReview(null) }} />}
      {respond != null && <RespondView target={respond} onClose={() => setRespond(null)} />}
      {message && <p className="muted g9-small" role="status"><Check size={12} aria-hidden="true" /> {message}</p>}
    </div>
  )
}

function LocalReviewCard() {
  const id = useId()
  const { data: personas } = useSWR('forge:personas', () => forgeApi.personas(), { revalidateOnFocus: false })
  const [persona, setPersona] = useState('general')
  const [source, setSource] = useState<'working' | 'staged' | 'range' | 'pr'>('working')
  const [arg, setArg] = useState('')
  const [result, setResult] = useState<ReviewResult | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async () => {
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      setResult(
        await forgeApi.review({
          persona,
          staged: source === 'staged',
          range: source === 'range' ? arg.trim() : null,
          pr: source === 'pr' ? arg.trim() : null,
        }),
      )
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="cdx-git-card k5-card" aria-labelledby={`${id}-t`}>
      <div id={`${id}-t`} className="cdx-git-subtitle"><ScanSearch size={13} aria-hidden="true" /> Kod incelemesi</div>
      <div className="ks-inline">
        <label htmlFor={`${id}-p`} className="sr-only">Persona</label>
        <select id={`${id}-p`} className="g7-select" value={persona} onChange={(e) => setPersona(e.target.value)} data-kor-cmd="forge.personas">
          {(personas?.personas ?? [{ name: 'general', source: 'builtin', title: '' }]).map((p) => (
            <option key={p.name} value={p.name}>{p.source === 'builtin' ? p.name : `${p.name} (${p.source === 'project' ? 'proje' : 'kullanıcı'})`}</option>
          ))}
        </select>
        <label htmlFor={`${id}-s`} className="sr-only">Kaynak</label>
        <select id={`${id}-s`} className="g7-select" value={source} onChange={(e) => setSource(e.target.value as typeof source)}>
          <option value="working">Kaydedilmemiş değişiklikler</option>
          <option value="staged">Hazırlanmış (staged)</option>
          <option value="range">Commit aralığı</option>
          <option value="pr">Pull request</option>
        </select>
      </div>
      {(source === 'range' || source === 'pr') && (
        <>
          <label htmlFor={`${id}-a`} className="sr-only">{source === 'range' ? 'Aralık' : 'PR numarası ya da adresi'}</label>
          <input id={`${id}-a`} className="text-input" value={arg} onChange={(e) => setArg(e.target.value)} placeholder={source === 'range' ? 'main..HEAD' : '12 ya da PR adresi'} maxLength={300} spellCheck={false} translate="no" />
        </>
      )}
      <div className="ks-actions">
        <button type="button" className="btn small" onClick={() => void run()} disabled={busy || ((source === 'range' || source === 'pr') && !arg.trim())} data-kor-cmd="forge.review">
          <ScanSearch size={13} aria-hidden="true" /> {busy ? 'İnceleniyor…' : 'İncele'}
        </button>
      </div>
      {result && <ReviewResults result={result} target={source === 'pr' ? arg.trim() : null} />}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

export function ForgeSection() {
  const id = useId()
  const { data, error, mutate } = useSWR('forge:detect', () => forgeApi.detect(), { revalidateOnFocus: false })
  const [refresh, setRefresh] = useState(0)
  const { data: status, mutate: mutateStatus } = useSWR(data?.remote && data.branch ? 'forge:status' : null, () => forgeApi.prStatus(), { revalidateOnFocus: false })
  return (
    <>
      <section className="cdx-git-card k5-card" aria-labelledby={`${id}-t`} data-kor-cmd="forge.detect">
        <div id={`${id}-t`} className="cdx-git-subtitle"><GitPullRequest size={13} aria-hidden="true" /> Pull request</div>
        {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
        {!data && !error && <p className="muted g9-small">Forge algılanıyor…</p>}
        {data && !data.remote && <p className="muted g9-small">{data.error}</p>}
        {data?.remote && (
          <>
            <p className="muted g9-small" translate="no">
              {`${KIND_LABEL[data.remote.kind] ?? data.remote.kind} · ${data.remote.host} · ${data.remote.owner}/${data.remote.repo}`}
            </p>
            {status?.pr && (
              <button type="button" className="btn small" onClick={() => openUrl(status.pr!.url)} title={status.pr.url} data-kor-cmd="forge.prStatus">
                <ExternalLink size={13} aria-hidden="true" /> {`Bu dalın PR'ı: #${status.pr.number} (${status.pr.state})`}
              </button>
            )}
            <CreateCard onDone={() => { void mutate(); void mutateStatus(); setRefresh((n) => n + 1) }} />
            <PrListCard refreshKey={refresh} />
          </>
        )}
      </section>
      <LocalReviewCard />
    </>
  )
}
