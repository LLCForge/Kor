import { Brain, Eye, Wrench } from 'lucide-react'
import type { CatalogModel } from '../../core/model-catalog.ts'
import { api } from './api.ts'
import { capabilityList, formatScore, formatTokens, formatUsd, PRIVACY_LABELS, relativePercent, traitLabel } from './g7-format.ts'
import { uiLocale } from './ui-locale.ts'

export const AA_URL = 'https://artificialanalysis.ai/'

/** Artificial Analysis ücretsiz API koşulu: verinin gösterildiği her yerde atıf. */
export function AaAttribution({ compact = false }: { compact?: boolean }) {
  return (
    <span className="g7-attribution">
      {compact ? 'Puanlar: ' : 'Kalite puanları ve hız verisi: '}
      <button type="button" className="g7-inline-link" onClick={() => void api.openExternal(AA_URL).catch(console.error)}>
        Artificial Analysis
      </button>
    </span>
  )
}

export function ScoreBar({ label, value, max, hideLabel = false }: { label: string; value: number | null | undefined; max: number; hideLabel?: boolean }) {
  if (value == null) return null
  const pct = relativePercent(value, max)
  return (
    <span className={`g7-score${hideLabel ? ' no-label' : ''}`} title={`${label}: ${formatScore(value)}`}>
      <span className={hideLabel ? 'sr-only' : 'g7-score-label'}>{label}</span>
      <span className="g7-score-track" aria-hidden="true">
        <span className="g7-score-fill" style={{ width: `${pct}%` }} />
      </span>
      <span className="g7-score-value">{formatScore(value)}</span>
    </span>
  )
}

/** Satır rozetleri: sansürsüz, kodlama sırası, yetenek simgeleri, büyük bağlam. */
export function ModelBadges({ model, rank }: { model: CatalogModel; rank: number | undefined }) {
  return (
    <>
      {model.uncensored && <span className="badge">Sansürsüz</span>}
      {rank !== undefined && rank <= 3 && <span className={`badge g7-rank${rank === 1 ? ' top' : ''}`}>Kodlamada #{rank}</span>}
      {!model.supportsTools && <span className="badge neutral g7-warn">Araç yok</span>}
      {(model.contextTokens ?? 0) >= 1_000_000 && <span className="badge neutral">{formatTokens(model.contextTokens)}</span>}
      {model.beta && <span className="badge neutral">Beta</span>}
      <span className="g7-icons">
        {model.supportsTools && (
          <span title="Araç çağırma">
            <Wrench size={12} aria-hidden="true" />
            <span className="sr-only">Araç çağırma</span>
          </span>
        )}
        {model.vision && (
          <span title="Görsel girdi">
            <Eye size={12} aria-hidden="true" />
            <span className="sr-only">Görsel girdi</span>
          </span>
        )}
        {model.reasoning && (
          <span title="Akıl yürütme">
            <Brain size={12} aria-hidden="true" />
            <span className="sr-only">Akıl yürütme</span>
          </span>
        )}
      </span>
    </>
  )
}

interface ModelDetailProps {
  id?: string
  model: CatalogModel
  rank: number | undefined
  aliases: string[]
  maxScores: { coding: number; intelligence: number; math: number }
}

export function ModelDetail({ id, model, rank, aliases, maxScores }: ModelDetailProps) {
  const privacy = model.privacy ? PRIVACY_LABELS[model.privacy] : null
  const caps = capabilityList(model)
  return (
    <div id={id} className="g7-detail pop-in">
      {model.description && <p className="g7-detail-desc text-pretty">{model.description}</p>}
      <dl className="g7-dl">
        <div>
          <dt>Bağlam</dt>
          <dd>{formatTokens(model.contextTokens)} token</dd>
        </div>
        <div>
          <dt>En fazla çıktı</dt>
          <dd>{formatTokens(model.maxCompletionTokens)} token</dd>
        </div>
        <div>
          <dt>Girdi / Çıktı (1M)</dt>
          <dd>
            {formatUsd(model.price?.input ?? null)} / {formatUsd(model.price?.output ?? null)}
          </dd>
        </div>
        <div>
          <dt>Önbellek okuma / yazma</dt>
          <dd>
            {formatUsd(model.price?.cacheInput ?? null)} / {formatUsd(model.price?.cacheWrite ?? null)}
          </dd>
        </div>
        <div>
          <dt>Gizlilik</dt>
          <dd title={privacy?.desc}>{privacy ? `${privacy.label}: ${privacy.desc}` : 'Bilinmiyor'}</dd>
        </div>
        <div>
          <dt>Yetenekler</dt>
          <dd>{caps.length > 0 ? caps.join(', ') : 'Yalnızca metin'}</dd>
        </div>
        {model.traits.length > 0 && (
          <div>
            <dt>Katalog işaretleri</dt>
            <dd>{model.traits.map(traitLabel).join(', ')}</dd>
          </div>
        )}
        {aliases.length > 0 && (
          <div>
            <dt>Takma adlar</dt>
            <dd className="g7-mono">{aliases.join(', ')}</dd>
          </div>
        )}
        {model.deprecatedAt && (
          <div>
            <dt>Kullanımdan kalkıyor</dt>
            <dd className="error-text">{new Date(model.deprecatedAt).toLocaleDateString(uiLocale(), { day: 'numeric', month: 'long', year: 'numeric' })}</dd>
          </div>
        )}
      </dl>
      {model.aa ? (
        <div className="g7-detail-aa">
          <p className="g7-detail-head">
            Artificial Analysis: <span className="g7-mono">{model.aa.name}</span>
            {!model.aa.exact && <span className="badge neutral">yaklaşık eşleşme</span>}
          </p>
          <div className="g7-detail-scores">
            <ScoreBar label="Kodlama" value={model.aa.coding} max={maxScores.coding} />
            <ScoreBar label="Zeka" value={model.aa.intelligence} max={maxScores.intelligence} />
            <ScoreBar label="Matematik" value={model.aa.math} max={maxScores.math} />
          </div>
          <p className="g7-detail-foot">
            {rank !== undefined ? `Kodlama sırası: #${rank} · ` : ''}
            Hız: {model.aa.tokensPerSecond == null ? '—' : `${formatScore(model.aa.tokensPerSecond)} token/sn`} · İlk token:{' '}
            {model.aa.ttftSeconds == null ? '—' : `${formatScore(model.aa.ttftSeconds)} sn`} · <AaAttribution compact />
          </p>
        </div>
      ) : null}
    </div>
  )
}

export function maxScoresOf(models: readonly CatalogModel[]): { coding: number; intelligence: number; math: number } {
  let coding = 0
  let intelligence = 0
  let math = 0
  for (const m of models) {
    coding = Math.max(coding, m.aa?.coding ?? 0)
    intelligence = Math.max(intelligence, m.aa?.intelligence ?? 0)
    math = Math.max(math, m.aa?.math ?? 0)
  }
  return { coding, intelligence, math }
}
