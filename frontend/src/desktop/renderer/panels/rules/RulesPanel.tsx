// §28.3 kural uyum paneli (ADR 0063): çalışma ağacındaki değişiklikler yüklü kurallara karşı (AGENTS.md,
// CLAUDE.md, .kor/rules, diğer araçların kuralları); deterministik denetim her zaman, isteğe bağlı model
// değerlendirmesi düz metin kuralları için; yüklü kurallar; en çok ihlal edilenler ve hangi modelde.
// CLI: kor rules check|list|report. Ajan aynı denetimi diff'i göstermeden önce yapar (onay kartı rozeti).
import { ListChecks, RefreshCw, Sparkles } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { RuleFinding } from '../../../ipc.gen.ts'
import { errorMessage } from '../../api.ts'
import { rulesApi } from '../../ek14-api.ts'
import { RulesBadge } from '../../rules/RulesBadge.tsx'
import { sortFindings } from '../../rules/findings.ts'
import type { KorPanelProps } from '../registry.ts'

export function RulesPanel(_: KorPanelProps) {
  const [staged, setStaged] = useState(false)
  const [result, setResult] = useState<{ findings: RuleFinding[]; files: number; judged: boolean; error: string | null } | null>(null)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const list = useSWR('rules:list', () => rulesApi.list(), { revalidateOnFocus: false })
  const report = useSWR('rules:report', () => rulesApi.report(7), { revalidateOnFocus: false })
  const check = (judge: boolean) => {
    setBusy(true)
    setErr(null)
    rulesApi
      .check(staged, judge)
      .then(setResult)
      .catch((e: unknown) => setErr(errorMessage(e)))
      .finally(() => setBusy(false))
  }
  return (
    <section className="ks" aria-label="Kural uyumu">
      <h2 className="ks-title">Kural uyumu</h2>
      <label className="ks-check"><input type="checkbox" checked={staged} onChange={(e) => setStaged(e.target.checked)} /> Yalnızca hazırlanmış (staged) değişiklikler</label>
      <div className="ks-actions">
        <button type="button" className="btn primary" disabled={busy} onClick={() => check(false)} data-kor-cmd="rules.check"><ListChecks size={14} aria-hidden="true" /> Denetle</button>
        <button type="button" className="btn" disabled={busy} onClick={() => check(true)} data-kor-cmd="rules.check" title="Düz metin kuralları yapılandırılmış modelle değerlendirilir (maliyeti kullanım defterine yazılır)">
          <Sparkles size={14} aria-hidden="true" /> Modelle de değerlendir
        </button>
      </div>
      {err && <p className="error-text" role="alert">{err}</p>}
      {busy && <p className="muted" role="status">Denetleniyor…</p>}
      {result && (
        <>
          <p className="ks-label">{result.files} dosya{result.judged ? ' · model değerlendirdi' : ''}</p>
          {result.error && <p className="ks-warn">{result.error}</p>}
          {result.files === 0 ? <p className="ks-empty">Denetlenecek değişiklik yok.</p> : result.findings.length === 0 ? <p className="ks-empty">Değişen dosyalara uygulanan kural yok.</p> : <RulesBadge findings={sortFindings(result.findings)} />}
        </>
      )}

      <div className="kg-card-head">
        <h3 className="ks-title">Son 7 günde en çok ihlal edilenler</h3>
        <button type="button" className="action-btn" onClick={() => void report.mutate()} aria-label="Raporu yenile" title="Yenile" data-kor-cmd="rules.report"><RefreshCw size={16} aria-hidden="true" /></button>
      </div>
      {report.data && report.data.length === 0 && <p className="ks-empty">İhlal kaydı yok.</p>}
      {report.data && report.data.length > 0 && (
        <ul className="ks-list" data-kor-cmd="rules.report">
          {report.data.slice(0, 20).map((r) => (
            <li key={`${r.source}:${r.model}`} className="ks-row">
              <span className="ks-badge">{r.count}×</span>
              <span className="kg-link-meta" translate="no">{r.rule}</span>
              <code translate="no">{r.source}</code>
              <span className="ks-chip" translate="no">{r.model}</span>
            </li>
          ))}
        </ul>
      )}

      <details className="ks-more">
        <summary>Yüklü kurallar ({list.data?.length ?? '…'})</summary>
        {list.error && <p className="error-text" role="alert">{errorMessage(list.error)}</p>}
        <ul className="ks-list" data-kor-cmd="rules.list">
          {(list.data ?? []).map((r, i) => (
            <li key={`${r.source}:${r.line}:${i}`} className="ks-row">
              <span className="ks-chip">{r.checkable ? 'denetlenir' : 'model'}</span>
              <span className="kg-link-meta" translate="no">{r.rule}</span>
              <code translate="no">{r.source}:{r.line}</code>
            </li>
          ))}
        </ul>
      </details>
    </section>
  )
}
