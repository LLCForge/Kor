// [g12] Kenar çubuğu "Güvenlik" düğmesi: komut sandbox'ı (kullanıcı ayarı) ve etkin sohbetin araç başına izinleri.
import { ShieldCheck } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { ToolKind } from '../../../core/permissions.ts'
import { G12_TOOL_LEVELS, type G12ToolLevel } from '../../ipc-g12.ts'
import { api, errorMessage } from '../api.ts'
import { Modal } from '../NameDialog.tsx'

const LEVEL_LABELS: Record<G12ToolLevel, string> = {
  default: 'İzin moduna göre',
  ask: 'Her seferinde sor',
  allow: 'Bu sohbette izin ver',
  deny: 'Bu sohbette kapalı',
}
const KIND_LABELS: Record<ToolKind, string> = { read: 'okuma', edit: 'düzenleme', exec: 'çalıştırma' }

const levelLabel = (kind: ToolKind, level: G12ToolLevel) => (kind === 'read' && level === 'default' ? 'Serbest (okuma aracı)' : LEVEL_LABELS[level])

function SandboxSection() {
  const id = useId()
  const { data, error, mutate } = useSWR('g12:sandbox', () => api.g12GetSandbox())
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const toggle = async (enabled: boolean) => {
    setBusy(true)
    setActionError(null)
    try {
      await mutate(api.g12SetSandbox(enabled), { revalidate: false })
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="g12-section" aria-labelledby={`${id}-title`}>
      <h3 id={`${id}-title`} className="g12-heading">Komut sandbox'ı</h3>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted">Yükleniyor...</p>
      ) : (
        <>
          <label className="g12-check">
            <input
              type="checkbox"
              checked={data.enabled}
              // Kullanılamayan platformda açmak yalnızca tüm komutları durdururdu; açıksa kapatılabilir kalır.
              disabled={busy || (!data.enabled && !data.available)}
              onChange={(e) => void toggle(e.target.checked)}
            />
            Kabuk komutlarını çekirdek düzeyi izolasyonda çalıştır (ağ kapalı, yalnızca çalışma dizinine yazma)
          </label>
          <p className={data.available ? 'dialog-desc' : 'g12-warning'}>{data.message}</p>
          {data.enabled && !data.available && (
            <p className="g12-warning" role="alert">
              Sandbox açık ama bu platformda kurulamıyor: ajan şu an kabuk komutu çalıştıramaz.
            </p>
          )}
          <p className="dialog-desc">Ayar kullanıcı ayar dosyanıza yazılır; proje dosyaları bunu değiştiremez. Bir sonraki komuttan itibaren geçerlidir.</p>
        </>
      )}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}

function ToolScopesSection({ sessionId }: { sessionId: string | null }) {
  const id = useId()
  const { data, error, mutate } = useSWR(sessionId ? ['g12:tools', sessionId] : null, ([, sid]: [string, string]) => api.g12GetToolScopes(sid))
  const [actionError, setActionError] = useState<string | null>(null)

  const change = async (tool: string, value: string) => {
    const level = G12_TOOL_LEVELS.find((l) => l === value)
    if (!sessionId || !level) return
    setActionError(null)
    try {
      await mutate(api.g12SetToolScope(sessionId, tool, level), { revalidate: false })
    } catch (err) {
      setActionError(errorMessage(err))
    }
  }

  return (
    <section className="g12-section" aria-labelledby={`${id}-title`}>
      <h3 id={`${id}-title`} className="g12-heading">Bu sohbetin araç izinleri</h3>
      <p className="dialog-desc">
        Her araca yalnızca bu sohbette geçerli ayrı yetki verilir; hiçbir araç kendiliğinden ek izin almaz. "Her seferinde sor" ve "kapalı" izin
        modundan (izinleri atla dahil) önce gelir. İzinler bellekte tutulur, uygulama kapanınca sıfırlanır.
      </p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !sessionId || data === null ? (
        <p className="muted">Sohbet henüz başlamadı; araç izinleri ilk mesajdan sonra ayarlanabilir.</p>
      ) : !data ? (
        <p className="muted">Yükleniyor...</p>
      ) : (
        <ul className="g12-tools">
          {data.map((tool) => (
            <li key={tool.name} className="g12-tool-row">
              <label htmlFor={`${id}-${tool.name}`} className="g12-tool-name">
                <code>{tool.name}</code> <span className="muted">{KIND_LABELS[tool.kind]}</span>
              </label>
              <select id={`${id}-${tool.name}`} className="g12-select" value={tool.level} onChange={(e) => void change(tool.name, e.target.value)}>
                {G12_TOOL_LEVELS.filter((l) => tool.kind !== 'read' || l !== 'allow').map((l) => (
                  <option key={l} value={l}>
                    {levelLabel(tool.kind, l)}
                  </option>
                ))}
              </select>
            </li>
          ))}
        </ul>
      )}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}

export function SecurityRailButton({ sessionId, collapsed }: { sessionId: string | null; collapsed: boolean }) {
  const [open, setOpen] = useState(false)
  const titleId = useId()
  const label = 'Güvenlik'
  return (
    <>
      <button type="button" className="rail-btn" onClick={() => setOpen(true)} title={collapsed ? label : undefined} aria-label={collapsed ? label : undefined}>
        <ShieldCheck size={16} aria-hidden="true" />
        <span className="rail-btn-label">{label}</span>
      </button>
      {open && (
        <Modal onClose={() => setOpen(false)} labelledBy={titleId}>
          <div className="dialog-body g12-security">
            <h2 id={titleId} className="dialog-title">Güvenlik</h2>
            <SandboxSection />
            <ToolScopesSection sessionId={sessionId} />
            <div className="dialog-actions">
              <span />
              <button type="button" className="btn" onClick={() => setOpen(false)}>
                Kapat
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  )
}
