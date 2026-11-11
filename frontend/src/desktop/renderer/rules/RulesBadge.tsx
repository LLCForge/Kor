// §28.3 kural uyum rozeti (ADR 0063): onay kartında diff'ten önce, uygulanan (yeşil), ilgili ama
// denetlenemeyen (sarı) ve ihlal edilen (kırmızı) kurallar, kaynak dosya:satır ile. Ajan kırmızı
// ihlalde düzenlemeyi önce kendisi iki kez yeniler; bu kart yalnızca hâlâ ihlal varsa ya da onay
// gerektiğinde görünür. Kullanıcı "yine de yaz" (kabul) ya da reddet ile karar verir.
import { CircleAlert, CircleCheck, CircleDot } from 'lucide-react'
import type { RuleFinding } from '../../ipc.gen.ts'
import { countFindings, findingLocation } from './findings.ts'
import './rules.css'

const ICON = { violated: CircleAlert, relevant: CircleDot, applied: CircleCheck } as const
const LABEL = { violated: 'ihlal', relevant: 'ilgili', applied: 'uyuldu' } as const

export function RulesBadge({ findings }: { findings: readonly RuleFinding[] }) {
  const n = countFindings(findings)
  return (
    <details className={`kr-rules${n.violated > 0 ? ' bad' : ''}`} open={n.violated > 0}>
      <summary>
        Kurallar: <span className="kr-rule-n violated">{n.violated} ihlal</span> · <span className="kr-rule-n relevant">{n.relevant} ilgili</span> ·{' '}
        <span className="kr-rule-n applied">{n.applied} uyuldu</span>
      </summary>
      <ul className="kr-rule-list">
        {findings.map((f, i) => {
          const Icon = ICON[f.verdict]
          return (
            <li key={`${f.source}:${f.line}:${i}`} className={`kr-rule ${f.verdict}`}>
              <Icon size={14} aria-label={LABEL[f.verdict]} />
              <span className="kr-rule-text" translate="no">{f.rule}</span>
              <code className="kr-rule-src" translate="no">{f.source}{f.line > 0 ? `:${f.line}` : ''}</code>
              {findingLocation(f) && <span className="kr-rule-at" translate="no">{findingLocation(f)}</span>}
              {f.message && <span className="kr-rule-msg" translate="no">{f.message}</span>}
              {!f.deterministic && <span className="kr-rule-tag">model</span>}
            </li>
          )
        })}
      </ul>
    </details>
  )
}
