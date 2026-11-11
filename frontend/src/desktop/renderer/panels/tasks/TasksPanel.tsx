// Çok ajanlı görevler (§8.2, ADR 0037): yeni görev, görev listesi, alt ajan kartları (rol, model, durum,
// maliyet, dal) ve çakışma görünümü (dosya bazında bizim/onların farkı, yeniden planla ya da bırak).
// Canlı güncelleme `kor:tasks/progress` ile gelir.
import { Ban, Bot, GitBranch, GitFork, GitMerge, Play, RefreshCw, Trash2, Wand2 } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { SubTask, TaskConflict, TaskState } from '../../../ipc-tasks.ts'
import { errorMessage } from '../../api.ts'
import { KorDots, KorSpinner } from '../../brand/brand.ts'
import { DiffView } from '../../DiffView.tsx'
import type { KorPanelProps } from '../registry.ts'
import { tasksApi } from './bridge.ts'
import { SUB_STATUS, TASK_STATUS, anyEstimated, isActive, modelText, parseModels, percent, totalUsd, upsert, usd } from './shape.ts'

const LIST_KEY = 'tasks:list'

function StartForm({ onStarted }: { onStarted: (id: string) => void }) {
  const id = useId()
  const [goal, setGoal] = useState('')
  const [agents, setAgents] = useState(5)
  const [budget, setBudget] = useState('')
  const [test, setTest] = useState('')
  const [models, setModels] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    const parsed = parseModels(models)
    if (parsed.bad.length) {
      setError(`Anlaşılmayan model ataması: ${parsed.bad.join(', ')} (biçim: rol=sağlayıcı/model)`)
      return
    }
    const b = budget.trim() ? Number(budget) : undefined
    if (b !== undefined && (!Number.isFinite(b) || b < 0)) {
      setError('Bütçe sıfır ya da pozitif bir sayı olmalı')
      return
    }
    setBusy(true)
    try {
      const r = await tasksApi.start({ goal, agents, budgetUsd: b, test: test || undefined, models: parsed.models })
      setGoal('')
      onStarted(r.id)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="kg-card kt-form" onSubmit={(e) => void submit(e)} aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><Bot size={16} aria-hidden="true" />Yeni ekip görevi</h3>
      <label htmlFor={`${id}-goal`} className="ks-label">Hedef</label>
      <textarea id={`${id}-goal`} className="kt-goal" rows={3} maxLength={8000} required value={goal} onChange={(e) => setGoal(e.target.value)} placeholder="ör. auth sistemini sıfırdan yaz" />
      <div className="kt-row">
        <label className="kt-field">
          <span className="ks-label">Ajan sayısı</span>
          <input type="number" min={1} max={16} value={agents} onChange={(e) => setAgents(Math.min(16, Math.max(1, Number(e.target.value) || 1)))} />
        </label>
        <label className="kt-field">
          <span className="ks-label">Bütçe (USD, boş = sınırsız)</span>
          <input type="number" min={0} step="0.01" value={budget} onChange={(e) => setBudget(e.target.value)} />
        </label>
      </div>
      <label className="kt-field">
        <span className="ks-label">Test komutu (her alt ajandan sonra ve birleşik dalda, sandbox içinde)</span>
        <input type="text" value={test} onChange={(e) => setTest(e.target.value)} placeholder="ör. cargo test" translate="no" />
      </label>
      <label className="kt-field">
        <span className="ks-label">Rol başına model (boş = maliyet yönlendiricisi seçer)</span>
        <input type="text" value={models} onChange={(e) => setModels(e.target.value)} placeholder="backend=openai/gpt-4.1, docs=local/qwen3" translate="no" />
      </label>
      {error && <p className="error-text" role="alert">{error}</p>}
      <div className="ko-actions">
        <button type="submit" className="btn small primary" disabled={busy || !goal.trim()} data-kor-cmd="tasks.start">
          <Play size={14} aria-hidden="true" />
          {busy ? 'Başlatılıyor…' : 'Ekibi başlat'}
        </button>
      </div>
    </form>
  )
}

function SubCard({ s }: { s: SubTask }) {
  return (
    <li className={`kt-sub ${s.status}`}>
      <div className="kt-sub-head">
        <strong translate="no">{s.index + 1}. {s.role}</strong>
        {/* §16.2 kural 5: görev başına KorDots (agent.task.progress → kor:tasks/progress) */}
        {s.status === 'running' && <KorDots width={18} />}
        <span className={`ks-badge${s.status === 'merged' ? ' on' : ''}`}>{SUB_STATUS[s.status]}</span>
      </div>
      <p className="kt-goal-text" translate="no">{s.goal}</p>
      <dl className="ks-facts">
        <dt>Model</dt>
        <dd translate="no">{modelText(s)}</dd>
        <dt>Maliyet</dt>
        <dd>{usd(s.cost_usd, s.estimated)}</dd>
        <dt>Dal</dt>
        <dd translate="no"><GitBranch size={12} aria-hidden="true" /> {s.branch}</dd>
        {s.files.length > 0 && (<><dt>Dosyalar</dt><dd translate="no">{s.files.join(', ')}</dd></>)}
        {s.test && (<><dt>Test</dt><dd>{s.test.ok ? 'geçti' : 'başarısız'}</dd></>)}
      </dl>
      {s.outside.length > 0 && <p className="ks-warn">Kendi alanı dışında değiştirdi: <span translate="no">{s.outside.join(', ')}</span></p>}
      {s.error && <p className="ks-broken" translate="no">{s.error}</p>}
      {s.review && s.status === 'rejected' && <p className="ks-warn" translate="no">{s.review}</p>}
    </li>
  )
}

function ConflictView({ task, c, onError }: { task: TaskState; c: TaskConflict; onError: (e: string) => void }) {
  const [busy, setBusy] = useState(false)
  const act = async (action: 'replan' | 'drop') => {
    setBusy(true)
    try {
      await tasksApi.resolve(task.id, c.index, action)
    } catch (err) {
      onError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="kg-card kt-conflict" aria-label={`${c.role} çakışması`}>
      <h4 className="kg-card-title ks-title"><GitMerge size={16} aria-hidden="true" />Çakışma: <span translate="no">{c.index + 1}. {c.role}</span></h4>
      <p className="muted ks-note">Birleştirme geri alındı; görev dalı yarım birleşmiş değil. Solda görev dalındaki hâl, sağda bu ajanın hâli.</p>
      {c.files.map((f) => (
        <div key={f.path} className="kt-file">
          <code translate="no">{f.path}</code>
          <DiffView change={{ path: f.path, before: f.ours, after: f.theirs }} mode="split" reviewable={false} />
        </div>
      ))}
      <details>
        <summary>git'in gösterdiği çakışma</summary>
        <pre className="kt-diff" translate="no">{c.diff}</pre>
      </details>
      <div className="ko-actions">
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void act('replan')} data-kor-cmd="tasks.resolve">
          <Wand2 size={14} aria-hidden="true" />Yeniden planla
        </button>
        <button type="button" className="btn small danger" disabled={busy} onClick={() => void act('drop')} data-kor-cmd="tasks.resolve">
          <Trash2 size={14} aria-hidden="true" />Bu parçayı bırak
        </button>
      </div>
    </section>
  )
}

function TaskDetail({ task, onSelect }: { task: TaskState; onSelect: (id: string) => void }) {
  const [error, setError] = useState<string | null>(null)
  const run = async (f: () => Promise<unknown>) => {
    setError(null)
    try {
      await f()
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const active = isActive(task.status)
  return (
    <section className="kg-card" aria-label={`Görev #${task.id}`}>
      <div className="kg-card-head">
        <h3 className="kg-card-title ks-title">#{task.id} <span translate="no">{task.goal}</span></h3>
        {active && <KorSpinner size={14} />}
        <span className={`ks-badge${task.status === 'done' ? ' on' : ''}`}>{TASK_STATUS[task.status]}</span>
      </div>
      <progress className="kt-progress" max={100} value={percent(task)} aria-label="İlerleme" />
      <dl className="ks-facts">
        <dt>Görev dalı</dt>
        <dd translate="no">{task.branch}</dd>
        <dt>Toplam maliyet</dt>
        <dd>{usd(totalUsd(task), anyEstimated(task))} (planlama {usd(task.orchestrator_usd, task.estimated)}, inceleme {usd(task.reviewer_usd, task.estimated)})</dd>
        {task.test && (<><dt>Birleşik dalda test</dt><dd>{task.test.ok ? 'geçti' : 'başarısız'}</dd></>)}
        {task.forked_from && (<><dt>Çatallandığı görev</dt><dd>#{task.forked_from}</dd></>)}
      </dl>
      {task.summary && <p className="muted" translate="no">{task.summary}</p>}
      {task.error && <p className="error-text" role="alert" translate="no">{task.error}</p>}
      {task.status === 'planning' && !task.subtasks.length && <p className="muted" role="status">Orkestratör görevi alt görevlere bölüyor…</p>}
      <ul className="kt-subs">{task.subtasks.map((s) => <SubCard key={s.index} s={s} />)}</ul>
      {task.conflicts.map((c) => <ConflictView key={c.index} task={task} c={c} onError={setError} />)}
      {task.test && !task.test.ok && <pre className="kt-diff" translate="no">{task.test.output}</pre>}
      <div className="ko-actions">
        {!active && task.status !== 'done' && task.status !== 'conflict' && (
          <button type="button" className="btn small" onClick={() => void run(() => tasksApi.resume(task.id))} data-kor-cmd="tasks.resume">
            <Play size={14} aria-hidden="true" />Devam et
          </button>
        )}
        <button type="button" className="btn small" onClick={() => void run(async () => onSelect((await tasksApi.fork(task.id)).id))} data-kor-cmd="tasks.fork">
          <GitFork size={14} aria-hidden="true" />Çatalla
        </button>
        {task.status !== 'cancelled' && task.status !== 'done' && (
          <button type="button" className="btn small danger" onClick={() => void run(() => tasksApi.cancel(task.id))} data-kor-cmd="tasks.cancel">
            <Ban size={14} aria-hidden="true" />İptal et
          </button>
        )}
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

export function TasksPanel(_props: KorPanelProps) {
  const { data, error, isLoading, mutate } = useSWR(LIST_KEY, () => tasksApi.list())
  const [selected, setSelected] = useState<string | null>(null)

  useEffect(
    () => tasksApi.onProgress((t) => void mutate((list) => upsert(list ?? [], t), { revalidate: false })),
    [mutate],
  )

  const current = data?.find((t) => t.id === selected) ?? data?.[0] ?? null

  return (
    <div className="ks" data-kor-event="kor:tasks/progress">
      <div className="kg-card-head">
        <h2 className="ks-title">Ekip görevleri</h2>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Görev listesini yenile" title="Yenile" data-kor-cmd="tasks.list">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      <StartForm onStarted={(id) => { setSelected(id); void mutate() }} />
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading || !data ? (
        <p className="muted" role="status" data-kor-cmd="tasks.list">Görevler yükleniyor…</p>
      ) : !data.length ? (
        <p className="ks-empty">Bu depoda henüz ekip görevi yok. Bir hedef yazın; orkestratör onu alt ajanlara böler, her biri kendi git çalışma ağacında çalışır.</p>
      ) : (
        <>
          <label className="kt-field">
            <span className="ks-label">Görev</span>
            <select className="g7-select" value={current?.id ?? ''} onChange={(e) => setSelected(e.target.value)} data-kor-cmd="tasks.status">
              {data.map((t) => (
                <option key={t.id} value={t.id}>#{t.id} · {TASK_STATUS[t.status]} · %{percent(t)} · {t.goal.slice(0, 60)}</option>
              ))}
            </select>
          </label>
          {current && <TaskDetail task={current} onSelect={(id) => { setSelected(id); void mutate() }} />}
        </>
      )}
    </div>
  )
}
