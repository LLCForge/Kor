// Karşılama / boş durum (spec §23.3, §23.4, ADR 0053): günün saatine göre selam + proje adı, ember kor işareti,
// proje yoksa tek eylem, proje varsa graf/analiz/git'ten türetilen çipler, gerçek sürüm/kanal rozeti
// (tıklanınca sürüm notları). Metin içermeyen boş ekran yoktur.
import { BookOpen, Bug, KeyRound, FlaskConical, FolderOpen, GitCommitHorizontal, GitMerge, ListChecks, ShieldAlert, Sparkles, Wand2 } from 'lucide-react'
import { type ReactNode } from 'react'
import useSWR from 'swr'
import type { AnalysisApi } from '../ipc-analysis.ts'
import { api, errorMessage } from './api.ts'
import { greetingWord, homeChips, projectName, releaseLabel, type ChipId } from './home.ts'
import { Chip, toast } from './kor-ui/index.ts'
import { Logo } from './Logo.tsx'
import { openG7Dialog } from './g7-bus.ts'

const ICONS: Record<ChipId, ReactNode> = {
  openFolder: <FolderOpen size={14} aria-hidden="true" />,
  sample: <Sparkles size={14} aria-hidden="true" />,
  explain: <BookOpen size={14} aria-hidden="true" />,
  security: <ShieldAlert size={14} aria-hidden="true" />,
  review: <ListChecks size={14} aria-hidden="true" />,
  conflicts: <GitMerge size={14} aria-hidden="true" />,
  lastCommit: <GitCommitHorizontal size={14} aria-hidden="true" />,
  bugs: <Bug size={14} aria-hidden="true" />,
  tests: <FlaskConical size={14} aria-hidden="true" />,
  improve: <Wand2 size={14} aria-hidden="true" />,
}

const analysis = (api as unknown as { analysis: AnalysisApi }).analysis

export interface HomeEmptyProps {
  composer: ReactNode
  onSuggestion: (prompt: string) => void
  disabled: boolean
  root: string | null
  onOpenFolder: () => void
  onOpenSample: () => void
}

export function HomeEmpty({ composer, onSuggestion, disabled, root, onOpenFolder, onOpenSample }: HomeEmptyProps) {
  const { data: git } = useSWR(root ? ['home-git', root] : null, () => api.git.status(), { revalidateOnFocus: false })
  // Deterministik yerel denetim (ağ yok: noExternal); bulgu yoksa ya da çalışmazsa çip görünmez.
  const { data: security } = useSWR(root ? ['home-security', root] : null, () => analysis.security(true), { revalidateOnFocus: false, shouldRetryOnError: false })
  const { data: update } = useSWR('home-update', () => api.g16UpdateStatus(), { revalidateOnFocus: false, shouldRetryOnError: false })
  const chips = homeChips({
    root,
    isRepo: git?.isRepo ?? null,
    dirty: git?.dirty ?? 0,
    conflicts: git?.conflicts.length ?? 0,
    securityFindings: security ? security.report.findings.length : null,
  })
  const release = releaseLabel(update?.version ?? null, update?.channel)
  const notesUrl = update?.releaseUrl ?? null

  return (
    <div className="empty-state">
      <div className="empty-inner">
        <div className="empty-brand">
          {/* Ember renkli marka işareti (gri/sönük değil); boştayken KorThinking gibi hafifçe süzülür ve parlar
              (bob 3,6 s). Hareket kapalıyken (OS, Animasyonlar = Kapalı, düşük güç) durağan kalır. */}
          <span className="kor-home-mark">
            <Logo size={root ? 52 : 72} />
          </span>
          {release &&
            (notesUrl ? (
              <button
                type="button"
                className="kor-release-pill"
                onClick={() => void api.openExternal(notesUrl).catch((err: unknown) => toast(errorMessage(err), 'error'))}
                title="Sürüm notlarını aç"
                data-kor-cmd="openExternal"
              >
                <span translate="no">{release}</span>
              </button>
            ) : (
              <span className="kor-release-pill" data-kor-cmd="g16UpdateStatus" title="Kurulu sürüm ve güncelleme kanalı">
                <span translate="no">{release}</span>
              </span>
            ))}
        </div>
        <h1 className="kor-greeting text-balance">
          {/* Selam ve proje adı ayrı metin düğümleri: çeviri selamı birebir eşleştirir, ad olduğu gibi kalır. */}
          <span>{greetingWord(new Date().getHours())}</span>
          {projectName(root) && <span translate="no">{` — ${projectName(root)}`}</span>}
        </h1>
        {!root && <p className="muted kor-empty-hint">Başlamak için bir proje klasörü aç ya da örnek projeyle dene. Klasörsüz de sohbet edebilirsin.</p>}
        {disabled && (
          <div className="kor-empty-hint kor-connect-card" role="note">
            <p className="muted text-pretty">Henüz bir model sağlayıcısı bağlı değil. Hesap gerekmez: bir API anahtarı ekleyin ya da bilgisayarınızdaki yerel modeli (Ollama, LM Studio) kullanın.</p>
            <button type="button" className="btn small primary" onClick={() => openG7Dialog({ kind: 'settings', section: 'providers' })} data-kor-cmd="g9.getProviders">
              <KeyRound size={14} aria-hidden="true" /> Sağlayıcı bağla
            </button>
          </div>
        )}
        {composer}
        <div className="suggestions kor-empty-action" role="group" aria-label="Hızlı eylemler">
          {chips.map((c) => (
            <Chip
              key={c.id}
              label={c.label}
              icon={ICONS[c.id]}
              disabled={disabled && c.prompt !== undefined}
              cmd={c.id === 'openFolder' ? 'openWorkspace' : c.id === 'sample' ? 'webview.sampleProject' : undefined}
              onClick={() => {
                if (c.id === 'openFolder') onOpenFolder()
                else if (c.id === 'sample') onOpenSample()
                else if (c.prompt) onSuggestion(c.prompt)
              }}
            />
          ))}
        </div>
      </div>
    </div>
  )
}
