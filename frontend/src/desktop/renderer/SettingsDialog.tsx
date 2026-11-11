import { Clock, Columns3, Cpu, FlaskConical, Info, Keyboard, Lock, Palette, Plus, Search, Settings, Shield, Sparkles, Trash2, Type, Workflow } from 'lucide-react'
import { useId, useMemo, useState, type FormEvent, type ReactNode } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { Logo } from './Logo.tsx'
import { Tooltip } from './kor-ui/index.ts'
import { MODEL_ALIAS_PATTERN, sortCatalog, TASK_KINDS, type TaskKind, type TaskModels } from '../../core/model-catalog.ts'
import type { ThemeSetting } from '../g7-types.ts'
import { api, errorMessage } from './api.ts'
import { chooseUiLanguage } from './i18n/boot.ts'
import { CATALOG_KEY, openG7Dialog, useCatalog, type SettingsSection } from './g7-bus.ts'
import { G7Modal } from './G7Modal.tsx'
import { AA_URL, AaAttribution } from './ModelDetail.tsx'
import { PermissionRules } from './PermissionRules.tsx'
import { PrivacyPanel } from './PrivacyPanel.tsx'
import { Blocks, Bot, Coins, Package, Plug, Puzzle } from 'lucide-react' // [g9]; Blocks: MCP; Package: eklenti; Bot: ajanlar; Puzzle: companion eklenti
import { CostSection } from './g9/CostSection.tsx' // [g9]
import { McpSection } from './mcp/McpSection.tsx'
import { PluginsSection } from './plugins/PluginsSection.tsx'
import { AgentsSection } from './agents/AgentsSection.tsx'
import { RoutinesSection } from './RoutinesSection.tsx'
import { ChromeExtensionSection } from './chrome/ChromeExtensionSection.tsx'
import { ConnectionsSection } from './connections/ConnectionsSection.tsx' // [g31]
import { Server } from 'lucide-react' // [g31]
import { ShieldCheck } from 'lucide-react'
import { CoreSection } from './core/CoreSection.tsx'
import { LayoutTemplate, TerminalSquare } from 'lucide-react' // [g5]
import { ShellSection } from './g5/ShellSection.tsx'
import { OsIntegrationSection } from './osint/OsIntegrationSection.tsx' // [osint] ADR 0052
import { MonitorCog } from 'lucide-react' // [osint]
import { TemplatesSection } from './g5/TemplatesSection.tsx'
import { CapabilitiesSettings, DesignSettings, MemorySettings, RuntimesPanel, SystemSettings, UsageSettings } from './settings-g27.tsx' // [g27]
import { TagsSection } from './TagsSection.tsx' // [cdx]
import { AutomationsSection, KeymapSection, ReadabilitySection } from './prefs/PrefsSections.tsx' // [ADR 0050]
import { MotionSetting } from './prefs/MotionSetting.tsx' // Görünüm > Animasyonlar
import { ProfilesSection } from './ProfilesSection.tsx' // [cdx] ajan profilleri
import { setG2Prefs, useG2Prefs } from './g2/prefs.ts' // [cdx] mesaj gönderme kısayolu
import { playNotificationSound, SOUND_OPTIONS } from './notify-sound.ts' // [cdx]
import type { EditorPrefs, GitPrefs, NotificationPrefs, ScriptPrefs } from '../g7-types.ts'
import { Volume2 } from 'lucide-react'
import { displayPath } from './display-path.ts'
import { findPanel } from './panels/registry.ts'
import { PanelBoundary } from './PanelBoundary.tsx'
import { uiLocale } from './ui-locale.ts'

const SETTINGS_KEY = 'g7:settings'

/** Yapılandırma niteliğindeki sağ panel görünümleri Ayarlar'da da bölüm olarak yaşar (bileşen olduğu gibi). */
const panelIcon = (id: string): ReactNode => {
  const Icon = findPanel(id)?.icon
  return Icon ? <Icon size={16} /> : null
}

interface SettingsDialogProps {
  open: boolean
  initialSection: SettingsSection | undefined
  onClose: () => void
  onOpenMemory: () => void
  onOpenSystemPrompt: () => void
  onSessionsCleared: () => void
  root: string | null
}

interface SectionDef {
  id: SettingsSection
  label: string
  icon: ReactNode
  /** Kenar çubuğunda hangi grup başlığı altında gösterilir. */
  group: string
  /** Arama bu sözcüklerde de yapılır */
  keywords: string
}

const GROUP_ORDER: readonly string[] = ['Tercihler', 'Bu bilgisayar', 'Özelleştirme']

const SECTIONS: readonly SectionDef[] = [
  { id: 'general', label: 'Genel', group: 'Tercihler', icon: <Settings size={16} aria-hidden="true" />, keywords: 'api anahtarı sistem istemi klasör' },
  { id: 'privacy', label: 'Gizlilik', group: 'Tercihler', icon: <Lock size={16} aria-hidden="true" />, keywords: 'gizlilik veri sil denetim' },
  { id: 'costs', label: 'Faturalandırma', group: 'Tercihler', icon: <Coins size={16} aria-hidden="true" />, keywords: 'maliyet fiyat tavan bütçe' },
  { id: 'usage', label: 'Kullanım', group: 'Tercihler', icon: <Info size={16} aria-hidden="true" />, keywords: 'kullanım sınır token bakiye' },
  { id: 'capabilities', label: 'Yetenekler', group: 'Tercihler', icon: <Sparkles size={16} aria-hidden="true" />, keywords: 'araç erişim modu görsel kod' },
  { id: 'memory', label: 'Bellek', group: 'Tercihler', icon: <Bot size={16} aria-hidden="true" />, keywords: 'bellek hafıza not' },
  { id: 'design', label: 'Tasarım sistemleri', group: 'Tercihler', icon: <Palette size={16} aria-hidden="true" />, keywords: 'tasarım renk yazı tipi' },
  { id: 'models', label: 'Değerlendirme', group: 'Tercihler', icon: <FlaskConical size={16} aria-hidden="true" />, keywords: 'model puan karşılaştırma' },
  { id: 'permissions', label: 'Zaman ve odak', group: 'Tercihler', icon: <Shield size={16} aria-hidden="true" />, keywords: 'izin kural allow deny onay' },
  { id: 'appearance', label: 'Görünüm', group: 'Tercihler', icon: <Palette size={16} aria-hidden="true" />, keywords: 'tema koyu açık animasyon hareket' },
  { id: 'readability', label: 'Okunabilirlik', group: 'Tercihler', icon: <Type size={16} aria-hidden="true" />, keywords: 'yazı boyutu satır genişliği yoğunluk kaydırma wrap diff erişilebilirlik ekran okuyucu karşıtlık hareket araç çıktısı' },
  { id: 'keymap', label: 'Tuş düzeni', group: 'Tercihler', icon: <Keyboard size={16} aria-hidden="true" />, keywords: 'kısayol klavye vim emacs keymap tuş' },
  { id: 'automations', label: 'Otomasyonlar', group: 'Tercihler', icon: <Workflow size={16} aria-hidden="true" />, keywords: 'otomatik auto sıkıştırma özet bellek güncelleme yönlendirme ses plan soru eşiği doctor düzelt' },
  { id: 'about', label: 'Kor', group: 'Tercihler', icon: <Info size={16} aria-hidden="true" />, keywords: 'sürüm hakkında' },
  { id: 'chrome', label: 'Tarayıcı', group: 'Tercihler', icon: <Puzzle size={16} aria-hidden="true" />, keywords: 'chrome tarayıcı eklenti' },
  { id: 'system', label: 'Sistem', group: 'Bu bilgisayar', icon: <Cpu size={16} aria-hidden="true" />, keywords: 'sürüm sistem tauri webview sağlık disk bellek ram cpu gpu pil temizlik önbellek docker bilgisayar kullanımı katman pencere düzeni' },
  { id: 'prereqs', label: 'Sistem gereksinimleri', group: 'Bu bilgisayar', icon: panelIcon('prereqs'), keywords: 'gereksinim kurulum docker podman wsl xcode webview2 izin erişilebilirlik ekran kaydı mikrofon winget brew apt paket yöneticisi' },
  { id: 'security', label: 'Güvenlik', group: 'Bu bilgisayar', icon: panelIcon('security'), keywords: 'güvenlik security sandbox denetim audit kayıt hash zincir kasa vault sır secret env' },
  { id: 'offline', label: 'Çevrimdışı', group: 'Bu bilgisayar', icon: panelIcon('offline'), keywords: 'offline çevrimdışı internetsiz air-gap usb paket yerel model llama.cpp ollama lm studio gguf' },
  { id: 'extensions', label: 'Uzantılar', group: 'Bu bilgisayar', icon: <Blocks size={16} aria-hidden="true" />, keywords: 'node python rust go java git docker kurulum araç' },
  { id: 'connections', label: 'Uzak bağlantılar', group: 'Bu bilgisayar', icon: <Server size={16} aria-hidden="true" />, keywords: 'ssh sunucu uzak veritabanı postgres mysql sqlite tünel' },
  { id: 'core', label: 'Çekirdek', group: 'Bu bilgisayar', icon: <ShieldCheck size={16} aria-hidden="true" />, keywords: 'gizlilik geri al undo uyumluluk compat ayar config pazar marketplace doctor' },
  { id: 'shell', label: 'Kabuk', group: 'Bu bilgisayar', icon: <TerminalSquare size={16} aria-hidden="true" />, keywords: 'kabuk shell bash zsh fish powershell entegrasyon fix why hata düzelt' },
  { id: 'osint', label: 'İşletim sistemi', group: 'Bu bilgisayar', icon: <MonitorCog size={16} aria-hidden="true" />, keywords: 'sağ tık menü kor ile aç explorer finder nautilus dolphin kısayol hotkey atlama listesi jump list dock kor:// bağlantı klasör ~/Kor sürükle bırak toplu düzenle widget powertoys spotlight' },
  { id: 'mcp', label: 'Geliştirici', group: 'Bu bilgisayar', icon: <Blocks size={16} aria-hidden="true" />, keywords: 'mcp sunucu geliştirici' },
  { id: 'mcpdev', label: 'MCP geliştirici', group: 'Bu bilgisayar', icon: panelIcon('mcpdev'), keywords: 'mcp dev inspector denetleyici sunucu araç tool json-rpc yeniden yükle' },
  { id: 'templates', label: 'Şablonlar', group: 'Özelleştirme', icon: <LayoutTemplate size={16} aria-hidden="true" />, keywords: 'şablon template görev run çalıştır istem prompt' },
  { id: 'routines', label: 'Rutinler', group: 'Özelleştirme', icon: <Clock size={16} aria-hidden="true" />, keywords: 'rutin zamanlanmış görev schedule routine otomasyon zamanlayıcı' },
  { id: 'agents', label: 'Ajanlar', group: 'Özelleştirme', icon: <Bot size={16} aria-hidden="true" />, keywords: 'ajan skill alt ajan' },
  { id: 'providers', label: 'Sağlayıcılar', group: 'Özelleştirme', icon: <Plug size={16} aria-hidden="true" />, keywords: 'sağlayıcı bağdaştırıcı anahtar api anahtarı hesap venice ollama sağlık katalog model skor proxy ağ geçidi bileşen medya' },
  { id: 'tools', label: 'Araçlar ve komutlar', group: 'Özelleştirme', icon: panelIcon('tools'), keywords: 'araç tool path cli komut satırı ffmpeg yardım tanım niyet intent öğret' },
  { id: 'hooks', label: "Hook'lar", group: 'Özelleştirme', icon: panelIcon('hooks'), keywords: 'hook hooks.toml settings.json olay event pre-edit post-edit pre-commit kuru çalıştır' },
  { id: 'compat', label: 'Uyumluluk', group: 'Özelleştirme', icon: panelIcon('compat'), keywords: 'compat uyumluluk cursor claude code windsurf codex copilot kural çakışma dışa aktar' },
  { id: 'team', label: 'Takım', group: 'Özelleştirme', icon: panelIcon('team'), keywords: 'team takım sunucu üye paylaşılan kural' },
  { id: 'plugins', label: 'Eklentiler', group: 'Özelleştirme', icon: <Package size={16} aria-hidden="true" />, keywords: 'eklenti plugin mağaza' },
]

const TASK_LABELS: Readonly<Record<TaskKind, { label: string; desc: string }>> = {
  explore: { label: 'Keşif (Explore)', desc: 'Kod tabanında arama yapan alt ajan' },
  plan: { label: 'Plan', desc: 'Uygulama planı çıkaran alt ajan' },
  review: { label: 'İnceleme (Review)', desc: 'Kod inceleme alt ajanı' },
  title: { label: 'Başlık üretimi', desc: 'Sohbet başlıklarını üreten istekler' },
  compaction: { label: 'Bağlam sıkıştırma', desc: 'Uzun sohbetleri özetleyen istekler' },
}

const THEMES: ReadonlyArray<{ id: ThemeSetting; label: string; desc: string }> = [
  { id: 'system', label: 'Sistem', desc: 'İşletim sisteminin açık/koyu ayarını izler' },
  { id: 'light', label: 'Açık', desc: 'Krem arka plan' },
  { id: 'dark', label: 'Koyu', desc: 'Koyu kahve arka plan' },
]

const openLink = (url: string) => void api.openExternal(url).catch((err: unknown) => console.error('Bağlantı açılamadı:', err))

export function SettingsDialog(props: SettingsDialogProps) {
  const titleId = useId()
  const [section, setSection] = useState<SettingsSection>(props.initialSection ?? 'general')
  const [query, setQuery] = useState('')

  const q = query.trim().toLocaleLowerCase('tr')
  const matches = useMemo(() => SECTIONS.filter((s) => !q || `${s.label} ${s.keywords}`.toLocaleLowerCase('tr').includes(q)), [q])
  const shown = q ? matches : SECTIONS.filter((s) => s.id === section)

  const render = (id: SettingsSection): ReactNode => {
    switch (id) {
      case 'general':
        return <GeneralSection root={props.root} onOpenSystemPrompt={props.onOpenSystemPrompt} />
      case 'usage':
        return <UsageSettings />
      case 'capabilities':
        return <CapabilitiesSettings />
      case 'memory':
        return <MemorySettings onOpen={props.onOpenMemory} />
      case 'design':
        return <DesignSettings />
      case 'system':
        return (
          <>
            <SystemSettings />
            <PanelSection id="system" />
          </>
        )
      case 'prereqs':
      case 'security':
      case 'offline':
      case 'mcpdev':
      case 'tools':
      case 'hooks':
      case 'compat':
      case 'team':
        return <PanelSection id={id} />
      case 'extensions':
        return <RuntimesPanel />
      case 'models':
        return <ModelsSection />
      case 'permissions':
        return <PermissionRules root={props.root} />
      case 'appearance':
        return <AppearanceSection />
      case 'readability':
        return <ReadabilitySection />
      case 'keymap':
        return <KeymapSection />
      case 'automations':
        return <AutomationsSection />
      case 'privacy':
        return <PrivacyPanel onSessionsCleared={props.onSessionsCleared} />
      case 'about':
        return <AboutSection />
      case 'providers': // [g9] Bağdaştırıcılar + Sağlayıcılar paneli tek bölüm (Anahtarlar sekmesi = ProvidersSection)
        return <PanelSection id="providers" />
      case 'costs': // [g9]
        return <CostSection />
      case 'mcp':
        return <McpSection />
      case 'plugins':
        return <PluginsSection />
      case 'agents':
        return <AgentsSection />
      case 'chrome':
        return <ChromeExtensionSection />
      case 'core':
        return <CoreSection />
      case 'connections':
        return <ConnectionsSection />
      case 'routines':
        return <RoutinesSection root={props.root} />
      case 'shell':
        return <ShellSection />
      case 'osint':
        return <OsIntegrationSection />
      case 'templates':
        return <TemplatesSection />
    }
  }

  return (
    <G7Modal open={props.open} onClose={props.onClose} titleId={titleId} title="Ayarlar" className="g7-settings">
      <div className="g7-settings-body">
        <nav className="g7-settings-nav" aria-label="Ayar bölümleri">
          <div className="g7-search">
            <Search size={14} aria-hidden="true" />
            <label htmlFor={`${titleId}-q`} className="sr-only">Ayarlarda ara</label>
            <input id={`${titleId}-q`} className="g7-search-input" placeholder="Ara" value={query} onChange={(e) => setQuery(e.target.value)} />
          </div>
          {/* Dar diyalogda bölüm listesi bu seçiciye iner (styles/kor-design.css, kor-settings kapsayıcısı). */}
          <label htmlFor={`${titleId}-sec`} className="sr-only">Ayar bölümleri</label>
          <select
            id={`${titleId}-sec`}
            className="text-input g7-nav-select"
            value={section}
            onChange={(e) => {
              setSection(e.target.value as SettingsSection)
              setQuery('')
            }}
          >
            {GROUP_ORDER.map((group) => (
              <optgroup key={group} label={group}>
                {SECTIONS.filter((s) => s.group === group).map((s) => (
                  <option key={s.id} value={s.id}>{s.label}</option>
                ))}
              </optgroup>
            ))}
          </select>
          {GROUP_ORDER.map((group) => {
            const items = SECTIONS.filter((s) => s.group === group && (!q || matches.includes(s)))
            if (items.length === 0) return null
            return (
              <div key={group} className="g7-nav-group-wrap">
                <div className="g7-nav-group">{group}</div>
                {items.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className={`g7-nav-btn${!q && section === s.id ? ' active' : ''}`}
                    aria-current={!q && section === s.id ? 'page' : undefined}
                    onClick={() => {
                      setSection(s.id)
                      setQuery('')
                    }}
                  >
                    {s.icon}
                    {s.label}
                  </button>
                ))}
              </div>
            )
          })}
        </nav>
        <div className="g7-settings-content">
          {shown.length === 0 && <p className="muted">"{query}" için ayar bulunamadı.</p>}
          {shown.map((s) => (
            <section key={s.id} className="g7-settings-section" aria-labelledby={`${titleId}-${s.id}`}>
              <h3 id={`${titleId}-${s.id}`} className="g7-h3">{s.label}</h3>
              {render(s.id)}
            </section>
          ))}
        </div>
      </div>
    </G7Modal>
  )
}

/** Sağ panel görünümünü ayar bölümünde çizer; kendi hata sınırı ve kapsayıcı sorguları (kor-panel) vardır. */
function PanelSection({ id }: { id: string }) {
  const panel = findPanel(id)
  if (!panel) return null
  return (
    <PanelBoundary resetKey={id}>
      <div className="kor-panel kor-settings-panel" data-kor-panel={id}>
        <panel.component sessionId={null} />
      </div>
    </PanelBoundary>
  )
}

function Row({ title, desc, children }: { title: string; desc?: ReactNode; children: ReactNode }) {
  return (
    <div className="g7-setting">
      <div className="g7-setting-text">
        <span className="g7-setting-title">{title}</span>
        {desc && <span className="g7-note">{desc}</span>}
      </div>
      <div className="g7-setting-control">{children}</div>
    </div>
  )
}

function GeneralSection({ root, onOpenSystemPrompt }: { root: string | null; onOpenSystemPrompt: () => void }) {
  const prefs = useG2Prefs()
  const settings = useSWR(SETTINGS_KEY, () => api.g7GetSettings())
  const notif: NotificationPrefs = { desktop: true, sound: true, soundName: 'soft', ...settings.data?.notifications }
  const gitPrefs: GitPrefs = { branchPrefix: 'kor/', worktreeDir: '', ...settings.data?.git }
  const editorPrefs: EditorPrefs = { type: 'code', customCommand: '', sshHost: '', ...settings.data?.editor }
  const [notifError, setNotifError] = useState<string | null>(null)
  const setNotif = (patch: Partial<NotificationPrefs>) => {
    const next = { ...notif, ...patch }
    void api.g7SetNotifications(next).then(() => settings.mutate()).catch((err: unknown) => setNotifError(errorMessage(err)))
  }
  const [gitDraft, setGitDraft] = useState<GitPrefs | null>(null)
  const scriptPrefs: ScriptPrefs = { setup: '', cleanup: '', dev: '', ...settings.data?.scripts }
  const [scriptDraft, setScriptDraft] = useState<ScriptPrefs | null>(null)
  const scripts = scriptDraft ?? scriptPrefs
  const saveScripts = (next: ScriptPrefs) => {
    setScriptDraft(next)
    void api.g7SetScripts(next).then(() => settings.mutate()).catch((err: unknown) => setNotifError(errorMessage(err)))
  }
  const [editorDraft, setEditorDraft] = useState<EditorPrefs | null>(null)
  const git = gitDraft ?? gitPrefs
  const editor = editorDraft ?? editorPrefs
  const saveGit = (next: GitPrefs) => {
    setGitDraft(next)
    void api.g7SetGit(next).then(() => settings.mutate()).catch((err: unknown) => setNotifError(errorMessage(err)))
  }
  const saveEditor = (next: EditorPrefs) => {
    setEditorDraft(next)
    void api.g7SetEditor(next).then(() => settings.mutate()).catch((err: unknown) => setNotifError(errorMessage(err)))
  }
  const sendId = useId()
  return (
    <div className="g7-stack">
      <Row title="Sistem istemi" desc="Varsayılan talimatların yerine geçen kendi istemin.">
        <button type="button" className="btn small" onClick={onOpenSystemPrompt}>Düzenle</button>
      </Row>
      <Row title="Açık klasör" desc={<span className="g7-mono g7-path">{root ? displayPath(root) : 'Klasör açılmadı'}</span>}>
        <span />
      </Row>
      {/* [cdx] cdesktop: Message Input — gönderme kısayolu */}
      <Row title="Mesaj girişi" desc="Mesajı hangi tuşla göndereceğini seç; diğeri yeni satır ekler.">
        <select id={sendId} className="g7-select" value={prefs.sendKey} onChange={(e) => setG2Prefs({ sendKey: e.target.value as 'enter' | 'ctrlEnter' })} aria-label="Gönderme kısayolu">
          <option value="enter">Enter gönderir (Shift+Enter yeni satır)</option>
          <option value="ctrlEnter">Ctrl/⌘+Enter gönderir (Enter yeni satır)</option>
        </select>
      </Row>
      {/* [cdx] cdesktop: Notifications — her tercih kendi satırında (anahtar sağda), ses seçimi ayrı satır */}
      <Row title="Masaüstü bildirimi" desc={notifError ?? 'Onay beklerken ve arka plan ajanı bitince işletim sistemi bildirimi.'}>
        <input type="checkbox" checked={notif.desktop} onChange={(e) => setNotif({ desktop: e.target.checked })} aria-label="Masaüstü bildirimi" />
      </Row>
      <Row title="Ses bildirimi" desc="Pencere odakta değilken ya da başka sohbetteyken kısa bir ses çalar.">
        <input type="checkbox" checked={notif.sound} onChange={(e) => setNotif({ sound: e.target.checked })} aria-label="Ses bildirimi" />
      </Row>
      <Row title="Bildirim sesi" desc="Hoparlör simgesiyle dinleyebilirsin.">
        <span className="cdx-sound-row">
          <select className="g7-select" value={notif.soundName} disabled={!notif.sound} onChange={(e) => setNotif({ soundName: e.target.value as NotificationPrefs['soundName'] })} aria-label="Bildirim sesi">
            {SOUND_OPTIONS.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
          </select>
          <Tooltip text="Sesi dinle"><button type="button" className="action-btn cdx-sound-play" onClick={() => playNotificationSound(notif.soundName)} disabled={!notif.sound} aria-label="Sesi dinle"><Volume2 size={15} aria-hidden="true" /></button></Tooltip>
        </span>
      </Row>
      {/* [cdx] cdesktop: Editor Type + Remote SSH */}
      <Row title="Editör" desc="“Editörde aç” hangi uygulamayı kullansın. Özel: komut ({file}, {line} yer tutucuları). Uzak SSH: VS Code ailesi --remote ssh-remote+host ile açar.">
        <span className="cdx-toggles cdx-editor-fields">
          <select className="g7-select" value={editor.type} onChange={(e) => saveEditor({ ...editor, type: e.target.value as EditorPrefs['type'] })} aria-label="Editör tipi">
            <option value="code">VS Code</option>
            <option value="cursor">Cursor</option>
            <option value="windsurf">Windsurf</option>
            <option value="zed">Zed</option>
            <option value="custom">Özel komut</option>
            <option value="system">Sistem varsayılanı</option>
          </select>
          {editor.type === 'custom' && (
            <input className="text-input" value={editor.customCommand} onChange={(e) => setEditorDraft({ ...editor, customCommand: e.target.value })} onBlur={() => saveEditor(editor)} placeholder="ör. subl {file}:{line}" spellCheck={false} aria-label="Özel editör komutu" />
          )}
          <input className="text-input" value={editor.sshHost} onChange={(e) => setEditorDraft({ ...editor, sshHost: e.target.value })} onBlur={() => saveEditor(editor)} placeholder="Uzak SSH host (isteğe bağlı, ör. dev-box)" spellCheck={false} aria-label="Uzak SSH host" />
        </span>
      </Row>
      {/* [cdx] cdesktop: Settings > Git — Branch Prefix + Workspace Directory */}
      <Row title="Git" desc={<span>Dal öneki: ajanın açtığı worktree dalları <code className="g7-mono">{git.branchPrefix || ''}1a2b-gorev-adi</code> biçiminde olur. Worktree klasörü boşsa deponun yanındaki <code className="g7-mono">&lt;depo&gt;-worktrees</code> kullanılır.</span>}>
        <span className="cdx-toggles cdx-editor-fields">
          <input className="text-input" value={git.branchPrefix} onChange={(e) => setGitDraft({ ...git, branchPrefix: e.target.value })} onBlur={() => saveGit(git)} placeholder="kor/" spellCheck={false} aria-label="Dal öneki" maxLength={40} />
          <input className="text-input" value={git.worktreeDir} onChange={(e) => setGitDraft({ ...git, worktreeDir: e.target.value })} onBlur={() => saveGit(git)} placeholder="Worktree klasörü (ör. ~/worktrees)" spellCheck={false} aria-label="Worktree klasörü" />
        </span>
      </Row>
      {/* [cdx] cdesktop: Setup / Cleanup / Dev Server scripts (proje .kor/settings.json) */}
      <Row title="Betikler" desc={<span>Kurulum: oturumda ajan başlamadan önce bir kez. Temizlik: her tur bitince. Dev sunucu: Web önizleme panelinden başlatılır. {root ? <>Şuraya kaydedilir: <code className="g7-mono">.kor/settings.json</code>; yalnızca güvenilen klasörde çalışır.</> : 'Klasör açılmadığından kullanıcı ayarına yazılır.'}</span>}>
        <span className="cdx-toggles cdx-editor-fields">
          <input className="text-input" value={scripts.setup} onChange={(e) => setScriptDraft({ ...scripts, setup: e.target.value })} onBlur={() => saveScripts(scripts)} placeholder="Kurulum betiği (ör. npm install)" spellCheck={false} aria-label="Kurulum betiği" />
          <input className="text-input" value={scripts.cleanup} onChange={(e) => setScriptDraft({ ...scripts, cleanup: e.target.value })} onBlur={() => saveScripts(scripts)} placeholder="Temizlik betiği (ör. npm run lint)" spellCheck={false} aria-label="Temizlik betiği" />
          <input className="text-input" value={scripts.dev} onChange={(e) => setScriptDraft({ ...scripts, dev: e.target.value })} onBlur={() => saveScripts(scripts)} placeholder="Dev sunucu betiği (ör. npm run dev)" spellCheck={false} aria-label="Dev sunucu betiği" />
        </span>
      </Row>
      {/* [perf] Denetim günlüğü saklama */}
      <Row title="Günlük saklama" desc="Araç kararları ~/.kor/audit altında gün dosyalarında tutulur; bu süreden eski olanlar (ve 200 MB üstü) açılışta silinir.">
        <select className="g7-select" value={String(settings.data?.logs.keepDays ?? 30)} onChange={(e) => void api.g7SetLogs({ keepDays: Number(e.target.value) }).then(() => settings.mutate()).catch((err: unknown) => setNotifError(errorMessage(err)))} aria-label="Günlük saklama süresi">
          <option value="7">7 gün</option>
          <option value="30">30 gün</option>
          <option value="90">90 gün</option>
          <option value="365">1 yıl</option>
        </select>
      </Row>
      <ProfilesSection />
      <TagsSection />
    </div>
  )
}

function ModelsSection() {
  const id = useId()
  const { mutate } = useSWRConfig()
  const settings = useSWR(SETTINGS_KEY, () => api.g7GetSettings())
  const catalog = useCatalog(true)
  const models = useMemo(() => sortCatalog(catalog.data?.models ?? [], 'name'), [catalog.data])
  const [alias, setAlias] = useState('')
  const [aliasTarget, setAliasTarget] = useState('')
  const [error, setError] = useState<string | null>(null)

  const aliases = settings.data?.modelAliases ?? {}
  const taskModels = settings.data?.taskModels ?? {}
  const aliasOk = MODEL_ALIAS_PATTERN.test(alias.trim().toLowerCase())

  const after = async () => {
    await Promise.all([settings.mutate(), mutate(CATALOG_KEY)])
  }

  const saveAliases = async (next: Record<string, string>) => {
    setError(null)
    try {
      await api.g7SetModelAliases(next)
      await after()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  const addAlias = (e: FormEvent) => {
    e.preventDefault()
    const name = alias.trim().toLowerCase()
    if (!aliasOk || !aliasTarget) return
    void saveAliases({ ...aliases, [name]: aliasTarget }).then(() => {
      setAlias('')
      setAliasTarget('')
    })
  }

  const removeAlias = (name: string) => void saveAliases(Object.fromEntries(Object.entries(aliases).filter(([a]) => a !== name)))

  const setTask = async (kind: TaskKind, value: string) => {
    setError(null)
    const next: TaskModels = { ...taskModels }
    if (value) next[kind] = value
    else delete next[kind]
    try {
      await api.g7SetTaskModels(next)
      await settings.mutate()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  const modelName = (modelId: string) => models.find((m) => m.id === modelId)?.name ?? modelId

  return (
    <div className="g7-stack">
      <div className="g7-button-row">
        <button type="button" className="btn small" onClick={() => openG7Dialog({ kind: 'compare' })}>
          <Columns3 size={14} aria-hidden="true" /> Modelleri karşılaştır
        </button>
        <button type="button" className="btn small" onClick={() => openG7Dialog({ kind: 'probe' })}>
          <FlaskConical size={14} aria-hidden="true" /> Model testi
        </button>
      </div>

      <section className="g7-subsection" aria-labelledby={`${id}-aa`}>
        <h4 id={`${id}-aa`} className="g7-h4">Artificial Analysis puanları</h4>
        <p className="g7-note text-pretty">
          Kodlama, zeka ve matematik indeksleri ile hız verisi Kor'un günlük güncellenen merkezi model kataloğundan gelir; anahtar gerekmez. Katalog bu
          bilgisayarda saklanır ve çevrimdışıyken de kullanılır. <AaAttribution compact />
        </p>
        {catalog.data && (
          <p className="g7-note">
            {catalog.data.aa.matched} / {catalog.data.models.length} model eşleşti
            {catalog.data.aa.fetchedAt ? ` · son güncelleme ${new Date(catalog.data.aa.fetchedAt).toLocaleString(uiLocale())}` : ''}
            {catalog.data.aa.message ? ` · ${catalog.data.aa.message}` : ''}
          </p>
        )}
      </section>

      <section className="g7-subsection" aria-labelledby={`${id}-alias`}>
        <h4 id={`${id}-alias`} className="g7-h4">Takma adlar</h4>
        <p className="g7-note">Model seçicide ve CLI'da <code>/model fast</code> gibi kullanılır. Görev modellerinde de takma ad seçebilirsin.</p>
        {Object.keys(aliases).length > 0 && (
          <ul className="g7-rule-list">
            {Object.entries(aliases)
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([name, target]) => (
                <li key={name} className="g7-rule">
                  <code>{name}</code>
                  <span className="g7-grow truncate">→ {modelName(target)}</span>
                  <button type="button" className="action-btn" onClick={() => removeAlias(name)} aria-label={`${name} takma adını sil`} title="Sil">
                    <Trash2 size={14} aria-hidden="true" />
                  </button>
                </li>
              ))}
          </ul>
        )}
        <form className="g7-inline" onSubmit={addAlias}>
          <label htmlFor={`${id}-alias-name`} className="sr-only">Takma ad</label>
          <input
            id={`${id}-alias-name`}
            className="text-input g7-alias-input"
            value={alias}
            onChange={(e) => setAlias(e.target.value)}
            placeholder="fast"
            maxLength={32}
            spellCheck={false}
            aria-invalid={alias.trim() !== '' && !aliasOk}
          />
          <label htmlFor={`${id}-alias-target`} className="sr-only">Model</label>
          <select id={`${id}-alias-target`} className="g7-select g7-grow" value={aliasTarget} onChange={(e) => setAliasTarget(e.target.value)}>
            <option value="">Model seç...</option>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name} ({m.id})
              </option>
            ))}
          </select>
          <button type="submit" className="btn" disabled={!aliasOk || !aliasTarget}>
            <Plus size={14} aria-hidden="true" /> Ekle
          </button>
        </form>
        {alias.trim() !== '' && !aliasOk && <p className="g7-note g7-warn-text">Küçük harfle başlamalı; yalnızca harf, rakam, - ve _ (en fazla 32).</p>}
      </section>

      <section className="g7-subsection" aria-labelledby={`${id}-task`}>
        <h4 id={`${id}-task`} className="g7-h4">Görev tipine göre model</h4>
        <p className="g7-note">Boş bırakılırsa ana model kullanılır. Alt ajan tanımında model yazıyorsa o önceliklidir. Açık sohbetlere hemen uygulanır.</p>
        {TASK_KINDS.map((kind) => (
          <div key={kind} className="g7-setting">
            <div className="g7-setting-text">
              <label htmlFor={`${id}-${kind}`} className="g7-setting-title">{TASK_LABELS[kind].label}</label>
              <span className="g7-note">{TASK_LABELS[kind].desc}</span>
            </div>
            <div className="g7-setting-control">
              <select id={`${id}-${kind}`} className="g7-select" value={taskModels[kind] ?? ''} onChange={(e) => void setTask(kind, e.target.value)}>
                <option value="">Ana model</option>
                {Object.keys(aliases).length > 0 && (
                  <optgroup label="Takma adlar">
                    {Object.keys(aliases).map((a) => (
                      <option key={a} value={a}>
                        {a}
                      </option>
                    ))}
                  </optgroup>
                )}
                <optgroup label="Modeller">
                  {taskModels[kind] && !models.some((m) => m.id === taskModels[kind]) && !Object.hasOwn(aliases, taskModels[kind] ?? '') && (
                    <option value={taskModels[kind]}>{taskModels[kind]}</option>
                  )}
                  {models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </optgroup>
              </select>
            </div>
          </div>
        ))}
      </section>
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

function AppearanceSection() {
  const id = useId()
  const settings = useSWR(SETTINGS_KEY, () => api.g7GetSettings())
  const [error, setError] = useState<string | null>(null)
  const current = settings.data?.theme ?? 'system'

  const choose = async (theme: ThemeSetting) => {
    setError(null)
    try {
      await api.g7SetTheme(theme)
      await settings.mutate()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  return (
    <fieldset className="g7-fieldset">
      <legend className="g7-setting-title">Tema</legend>
      <div className="g7-theme-options">
        {THEMES.map((t) => (
          <label key={t.id} className={`g7-theme${current === t.id ? ' active' : ''}`} htmlFor={`${id}-${t.id}`}>
            <input id={`${id}-${t.id}`} type="radio" name={`${id}-theme`} checked={current === t.id} onChange={() => void choose(t.id)} />
            <span className={`g7-theme-swatch ${t.id}`} aria-hidden="true" />
            <span className="g7-setting-title">{t.label}</span>
            <span className="g7-note">{t.desc}</span>
          </label>
        ))}
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      <LanguageSection />
      <MotionSetting />
    </fieldset>
  )
}

/** Dil adları kendi dillerinde yazılır ve çevrilmez (`translate="no"`); yalnız "Otomatik" çevrilir. */
const LANGUAGES = [
  { id: 'auto', label: 'Otomatik', native: false },
  { id: 'tr', label: 'Türkçe', native: true },
  { id: 'en', label: 'English', native: true },
  { id: 'fr', label: 'Français', native: true },
] as const

function LanguageSection() {
  const id = useId()
  const state = useSWR('ui-language', () => api.getState())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const current = state.data?.uiLanguagePref ?? 'auto'

  const choose = async (pref: (typeof LANGUAGES)[number]['id']) => {
    setError(null)
    setBusy(true)
    try {
      await chooseUiLanguage(pref)
      await state.mutate()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <fieldset className="g7-fieldset g7-lang-fieldset" disabled={busy}>
      <legend className="g7-setting-title">Dil</legend>
      <p className="g7-note">Arayüz ve Kor'un mesajları bu dilde gösterilir. Ayar terminaldeki kor ile ortaktır.</p>
      <div className="g7-lang-options">
        {LANGUAGES.map((l) => (
          <label key={l.id} className={`g7-lang${current === l.id ? ' active' : ''}`} htmlFor={`${id}-${l.id}`}>
            <input id={`${id}-${l.id}`} type="radio" name={`${id}-lang`} checked={current === l.id} onChange={() => void choose(l.id)} />
            <span className="g7-setting-title" translate={l.native ? 'no' : undefined}>
              {l.label}
            </span>
          </label>
        ))}
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
    </fieldset>
  )
}

function AboutSection() {
  const { data } = useSWR(SETTINGS_KEY, () => api.g7GetSettings())
  return (
    <div className="g7-stack">
      <div className="about-brand">
        <Logo size={44} />
        <div>
          <div className="about-name">Kor</div>
          <span className="brand-badge">Beta · Sürüme az kaldı</span>
        </div>
      </div>
      <dl className="g7-dl">
        <div>
          <dt>Sürüm</dt>
          <dd>{data?.app.version ?? '—'}</dd>
        </div>
        <div>
          <dt>Tauri</dt>
          <dd>{data?.app.tauri ?? '—'}</dd>
        </div>
        <div>
          <dt>{data?.app.webviewName ?? 'WebView'}</dt>
          <dd>{data?.app.webview || '—'}</dd>
        </div>
        <div>
          <dt>Platform</dt>
          <dd>{data ? `${data.app.platform} (${data.app.arch})` : '—'}</dd>
        </div>
        <div>
          <dt>Telemetri</dt>
          <dd>Yok. Uygulama kullanım verisi toplamaz.</dd>
        </div>
      </dl>
      <p className="g7-note text-pretty">
        Modeller ve fiyatlar sohbet API'sinden alınır. Model kalite puanları ve hız verisi{' '}
        <button type="button" className="g7-inline-link" onClick={() => openLink(AA_URL)}>
          Artificial Analysis
        </button>{' '}
        tarafından sağlanır (ücretsiz API, atıf zorunlu).
      </p>
    </div>
  )
}
