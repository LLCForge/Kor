import { toolsOpenByDefault } from './prefs/readability.ts' // [ADR 0050]
import { Bot, Check, ChevronDown, ChevronRight, FilePen, FileText, Globe, ListTodo, Search, ShieldAlert, SquareTerminal, TriangleAlert, Wrench, X, type LucideIcon } from 'lucide-react'
import { useState } from 'react'
import type { Approval } from '../../core/agent.ts'
import { ACTIVE_VERBS, formatDuration, toolMeta } from './activity.ts'
import { lineStats, TOOL_FAILURE, TOOL_STOPPED, type ChatItem, type FileEdit } from './chat-state.ts'
import { DiffView } from './DiffView.tsx'
import { LongOutput } from './LongOutput.tsx'

import { LangBadge } from './LangBadge.tsx'
import { ReviewCount } from './ReviewCount.tsx'
import { destructiveReason, type FileDecisions } from '../ipc-review.ts' // [review]
import { RulesBadge } from './rules/RulesBadge.tsx' // §28.3
import { ReviewPanel } from './review/ReviewPanel.tsx' // [review]

const FILE_TOOLS = new Set(['read_file', 'write_file', 'edit_file', 'multi_edit'])

/** Kart açılınca o çağrının her düzenlemesi için gerçek diff (Claude Code / cdesktop gibi). */
function EditDiffs({ edits }: { edits: readonly FileEdit[] }) {
  const [closed, setClosed] = useState<ReadonlySet<number>>(new Set())
  const toggle = (i: number) => setClosed((prev) => { const n = new Set(prev); if (n.has(i)) n.delete(i); else n.add(i); return n })
  return (
    <div className="tool-diffs">
      {edits.map((e, i) => {
        const s = lineStats(e.previous, e.after)
        const open = !closed.has(i)
        return (
          <div key={`${e.path}:${i}`} className="tool-diff">
            <div className="tool-diff-head">
              <button type="button" className="action-btn cdx-diff-collapse" onClick={() => toggle(i)} aria-expanded={open} aria-label={open ? 'Diff’i daralt' : 'Diff’i genişlet'} title={open ? 'Daralt' : 'Genişlet'}>
                {open ? <ChevronDown size={13} aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
              </button>
              <LangBadge path={e.path} />
              <span className="tool-diff-path" title={e.path}>{e.path}</span>
              {(s.added > 0 || s.removed > 0) && (
                <span className="line-stats">
                  <span className="stat-add">+{s.added}</span>
                  <span className="stat-del">−{s.removed}</span>
                </span>
              )}
              <ReviewCount path={e.path} />
            </div>
            {open && (
              <div className="tool-diff-body">
                <DiffView change={{ path: e.path, before: e.previous, after: e.after }} mode="unified" />
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

// Tamamlanan araç: geçmiş zaman fiili ("Okundu dosya.ts"); çalışan araç: ACTIVE_VERBS ("Okunuyor").
const TOOL_LABELS: Record<string, string> = {
  read_file: 'Okundu',
  write_file: 'Yazıldı',
  edit_file: 'Düzenlendi',
  multi_edit: 'Düzenlendi',
  list_files: 'Dosyalar arandı',
  use_skill: 'Skill çalıştırıldı',
  run_python: 'Python çalıştırıldı',
  powershell: 'Çalıştırıldı',
  tool_search: 'Araç arandı',
  apply_patch: 'Yama uygulandı',
  monitor_shell: 'İzleniyor',
  grep: 'Arandı',
  bash: 'Çalıştırıldı',
  bash_output: 'Çıktı okundu',
  kill_shell: 'Durduruldu',
  web_fetch: 'Getirildi',
  web_search: 'Web\'de arandı',
  remember: 'Hafızaya kaydedildi',
  mark_chapter: 'Bölüm',
  todo_write: 'Görev listesi',
  suggest_next: 'Sonraki adım',
  ask_user: 'Soru',
  exit_plan_mode: 'Plan',
}

type ToolItem = Extract<ChatItem, { kind: 'tool' }>

/** §23.4: her satırın başında aracın türü (ikon); durum renkle ve etiketle verilir. Çalışan araç dönmez:
 *  ekranda tek spinner kuralı (§23.1, §23.5) — canlı durum zaten etkinlik grubunun kıvılcımında. */
export function toolIcon(name: string): LucideIcon {
  if (/^(read|view|notebook_read|bash_output)/.test(name)) return FileText
  if (/^(write|edit|multi_edit|apply_patch|notebook_edit)/.test(name)) return FilePen
  if (/^(bash|powershell|run_python|monitor_shell|kill_shell)/.test(name)) return SquareTerminal
  if (/^(grep|glob|list_files|tool_search)/.test(name)) return Search
  if (/^(web_|browser)/.test(name)) return Globe
  if (name === 'agent') return Bot
  if (name === 'todo_write') return ListTodo
  return Wrench
}

function StatusIcon({ item }: { item: ToolItem }) {
  if (item.awaitingApproval) return <ShieldAlert size={14} className="tool-icon warn" aria-label="Onay bekliyor" />
  if (item.result !== null && TOOL_FAILURE.test(item.result)) return <X size={14} className="tool-icon fail" aria-label={item.result === TOOL_STOPPED ? 'Durduruldu' : 'Başarısız'} />
  const Icon = toolIcon(item.name)
  return <Icon size={14} className={`tool-icon${item.result === null ? ' running' : ''}`} aria-label={item.result === null ? 'Çalışıyor' : 'Tamamlandı'} />
}

interface ToolCardProps {
  item: ToolItem
  /** files: [review] dosya bazında kararlar (§12.3); yoksa düz yanıt */
  onApprove: (id: string, approval: Approval, files?: FileDecisions) => void
}

export function ToolCard({ item, onApprove }: ToolCardProps) {
  // §28.9 `ui.tool_output = verbose`: araç çıktıları açık gelir.
  const [open, setOpen] = useState(toolsOpenByDefault)
  const hasResult = item.result !== null
  const label = hasResult ? (TOOL_LABELS[item.name] ?? item.name) : (ACTIVE_VERBS[item.name] ?? TOOL_LABELS[item.name] ?? item.name)
  const meta = toolMeta(item)

  return (
    <div className={`tool${item.awaitingApproval ? ' pending' : ''}`}>
      <button
        type="button"
        className="tool-row"
        onClick={() => setOpen((o) => !o)}
        disabled={!hasResult}
        aria-expanded={hasResult ? open : undefined}
      >
        <StatusIcon item={item} />
        <span className="tool-label">{label}</span>
        {FILE_TOOLS.has(item.name) && <LangBadge path={item.summary.replace(/\s*\([^)]*\)$/, '')} />}
        <span className={`tool-summary${item.name === 'bash' ? ' mono' : ''}`} title={item.summary}>
          {item.summary}
        </span>
        {meta && <span className="tool-meta">{meta}</span>}
        {item.startedAt !== undefined && item.endedAt !== undefined && <span className="tool-meta">{formatDuration(item.endedAt - item.startedAt)}</span>}
        {(item.added > 0 || item.removed > 0) && (
          <span className="line-stats">
            <span className="stat-add">+{item.added}</span>
            <span className="stat-del">−{item.removed}</span>
          </span>
        )}
        {hasResult && <ChevronRight size={14} className={`tool-chevron${open ? ' open' : ''}`} aria-hidden="true" />}
      </button>

      {/* §28.3 kural uyumu: diff'ten önce, kaynak dosya:satır ile */}
      {item.awaitingApproval && item.rules && item.rules.length > 0 && <RulesBadge findings={item.rules} />}
      {item.awaitingApproval && item.files && item.files.length > 0 && (
        // [review] yazmadan önce dosya bazında kabul / düzenle / reddet
        <div className="approval" role="group" aria-label={`${label} için onay`}>
          <ReviewPanel toolId={item.id} files={item.files} destructive={item.destructive} onAnswer={(approval, files) => onApprove(item.id, approval, files)} />
        </div>
      )}
      {item.awaitingApproval && !item.files?.length && (
        <div className={`approval${item.destructive ? ' review danger' : ''}`} role="group" aria-label={`${label} için onay`}>
          {item.destructive && (
            <p className="review-danger" role="alert">
              <TriangleAlert size={14} aria-hidden="true" /> Yıkıcı işlem: {destructiveReason(item.destructive)}
            </p>
          )}
          {item.warning && (
            <p className="approval-warning">
              <TriangleAlert size={14} aria-hidden="true" /> {item.warning}
            </p>
          )}
          {item.preview ? (
            // cdesktop onay kartı: dosya başlığı + yapılacak değişikliğin gerçek diff'i, altında ✓ / ✕
            <>
              <p className="approval-text">
                Ajan <strong>{item.preview.previous === null ? 'dosya oluşturmak' : 'dosyayı düzenlemek'}</strong> istiyor:
              </p>
              <div className="approval-preview">
                <EditDiffs edits={[item.preview]} />
              </div>
            </>
          ) : (
            <>
              <p className="approval-text">
                Ajan <strong>{item.name}</strong> aracını çalıştırmak istiyor:
              </p>
              <pre className="approval-subject">{item.summary}</pre>
            </>
          )}
          <div className="approval-actions">
            <button type="button" className={`btn ${item.destructive ? 'danger' : 'primary approve'}`} data-kor-cmd="approve" onClick={() => onApprove(item.id, 'yes')}>
              <Check size={14} aria-hidden="true" /> İzin ver
            </button>
            {!item.warning && !item.destructive && (
              <button type="button" className="btn" onClick={() => onApprove(item.id, 'always')}>
                Bu oturumda hep izin ver
              </button>
            )}
            <button type="button" className="btn ghost deny" onClick={() => onApprove(item.id, 'no')}>
              <X size={14} aria-hidden="true" /> Reddet
            </button>
          </div>
        </div>
      )}

      {/* Düzenleme: o adımın gerçek diff'i (cdesktop/Claude Code); diğerleri: büyük çıktılar sayfa sayfa açılır */}
      {open && item.result !== null && (item.edits && item.edits.length > 0 ? <EditDiffs edits={item.edits} /> : <LongOutput text={item.result} />)}
    </div>
  )
}
