// [g8] ```html blokları: model üretimi HTML'i güvenli önizler. Gerçek güvenlik sınırı SandboxedPreview'dir
//      (opak origin + sandbox + CSP default-src/connect-src 'none'); bu bileşen onu DEĞİŞTİRMEZ, üstüne UI kurar:
//      önizleme/kaynak sekmesi (72), kart başına betik izni (73, SandboxedPreview.allowScriptsToggle),
//      genişlik ön ayarı (92), ayrı pencerede (uygulama içi modal) büyütme (79), dosyaya kaydet (84, g4 jailed API),
//      ve statik çözümleyiciden (rich-html-analyze) engellenen dış istek/WebGL/data:/a11y bilgisi (78/82/86/93).
//      AĞ HER ZAMAN KAPALIDIR (91): alan adına izin verilmez; nedeni bilgi panelinde açıklanır.
import { AlertTriangle, Code, Eye, FileCode, Globe, Info, Maximize2, Save, WifiOff } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { Modal, NameDialog } from './NameDialog.tsx'
import { SandboxedPreview } from './SandboxedPreview.tsx'
import { api } from './api.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { analyzeHtml, formatBytes } from './rich-html-analyze.ts'

type WidthPreset = 'desktop' | 'tablet' | 'mobile'

/** Önizleme genişliği ön ayarları (92). desktop = tam genişlik. */
const PRESETS: { id: WidthPreset; label: string; width: number | null }[] = [
  { id: 'desktop', label: 'Masaüstü', width: null },
  { id: 'tablet', label: 'Tablet', width: 768 },
  { id: 'mobile', label: 'Mobil', width: 375 },
]

function ModalHost({ titleId, title, code }: { titleId: string; title: string; code: string }) {
  // 79: ayrı OS penceresi yerine uygulama içi büyük modal. SandboxedPreview aynı güvenlik sınırını korur
  //     (opak origin, ağ yok); yeni bir pencere açmak renderer'a yeni yetenek/origin vermeden mümkün değildir.
  return (
    <>
      <h2 id={titleId} className="sr-only">
        {title}
      </h2>
      <div className="g8-expand-body">
        <SandboxedPreview content={code} kind="html" title={title} allowScriptsToggle height={640} />
      </div>
    </>
  )
}

export function HtmlBlock({ code }: RichBlockProps) {
  const [tab, setTab] = useState<'preview' | 'source'>('preview')
  const [preset, setPreset] = useState<WidthPreset>('desktop')
  const [showInfo, setShowInfo] = useState(false)
  const [expanded, setExpanded] = useState(false)
  const [saving, setSaving] = useState(false)
  const expandTitleId = useId()
  const analysis = useMemo(() => analyzeHtml(code), [code])

  const width = PRESETS.find((p) => p.id === preset)?.width ?? null
  const warnings =
    analysis.blockedRefs.length + (analysis.hasWebgl ? 1 : 0) + analysis.a11yIssues.length + (analysis.dataUriOverLimit ? 1 : 0) + (analysis.oversize ? 1 : 0)

  // 84: önizlenen HTML'i çalışma dizinine yazar. Ana sürecin mevcut jailed API'si (yol hapsi + hassas dosya reddi
  //     + var olan adı ezmeme) tüm güvenlik denetimini yapar; renderer ham yol görmez.
  const saveToFile = async (name: string): Promise<void> => {
    const rel = await api.g4.createFile('', name)
    await api.g4.writeFile(rel, code, null)
  }

  return (
    <div className="code-block g8-rich">
      <div className="code-head">
        <span className="g8-lang">
          <Code size={13} aria-hidden="true" /> html
        </span>
        <div className="g8-tools">
          <div className="g8-tabs" role="group" aria-label="Görünüm">
            <button type="button" className={`g8-tab${tab === 'preview' ? ' g8-on' : ''}`} onClick={() => setTab('preview')} aria-pressed={tab === 'preview'}>
              <Eye size={14} aria-hidden="true" /> Önizleme
            </button>
            <button type="button" className={`g8-tab${tab === 'source' ? ' g8-on' : ''}`} onClick={() => setTab('source')} aria-pressed={tab === 'source'}>
              <FileCode size={14} aria-hidden="true" /> Kaynak
            </button>
          </div>
          {tab === 'preview' && (
            <div className="g8-presets" role="group" aria-label="Genişlik">
              {PRESETS.map((p) => (
                <button key={p.id} type="button" className={`g8-preset${preset === p.id ? ' g8-on' : ''}`} onClick={() => setPreset(p.id)} aria-pressed={preset === p.id} title={p.label}>
                  {p.label}
                </button>
              ))}
            </div>
          )}
          <button type="button" className={`code-copy${showInfo ? ' g8-on' : ''}`} onClick={() => setShowInfo((s) => !s)} aria-pressed={showInfo} aria-label="Güvenlik ve içerik bilgisi" title="Güvenlik ve içerik bilgisi">
            <Info size={15} aria-hidden="true" />
          </button>
          <button type="button" className="code-copy" onClick={() => setExpanded(true)} aria-label="Ayrı pencerede büyüt" title="Ayrı pencerede büyüt">
            <Maximize2 size={15} aria-hidden="true" />
          </button>
          <button type="button" className="code-copy" onClick={() => setSaving(true)} aria-label="Dosyaya kaydet" title="Dosyaya kaydet">
            <Save size={15} aria-hidden="true" />
          </button>
          <CopyButton className="code-copy" label="HTML kodunu kopyala" getText={() => code} />
        </div>
      </div>

      {warnings > 0 && (
        <div className="g8-chips">
          {analysis.blockedRefs.length > 0 && (
            <span className="g8-chip" title="Bu önizlemede dış ağ erişimi kapalıdır (CSP connect-src 'none').">
              <Globe size={12} aria-hidden="true" /> {analysis.blockedRefs.length} dış istek engellendi
            </span>
          )}
          {analysis.hasWebgl && (
            <span className="g8-chip g8-chip-warn" title="WebGL yoğun kaynak kullanabilir; yalnızca betik iznini verdiğinizde çalışır.">
              <AlertTriangle size={12} aria-hidden="true" /> WebGL içeriği
            </span>
          )}
          {analysis.a11yIssues.length > 0 && (
            <span className="g8-chip g8-chip-warn" title="Erişilebilirlik uyarıları için bilgi panelini açın.">
              <AlertTriangle size={12} aria-hidden="true" /> {analysis.a11yIssues.length} erişilebilirlik uyarısı
            </span>
          )}
          {analysis.dataUriOverLimit && (
            <span className="g8-chip g8-chip-warn">
              <AlertTriangle size={12} aria-hidden="true" /> Büyük gömülü görsel
            </span>
          )}
          {analysis.oversize && (
            <span className="g8-chip g8-chip-warn">
              <AlertTriangle size={12} aria-hidden="true" /> İçerik çok büyük
            </span>
          )}
        </div>
      )}

      {tab === 'preview' ? (
        <div className="g8-preview-wrap" style={width ? { maxWidth: width } : undefined}>
          <SandboxedPreview content={code} kind="html" title="HTML önizleme" allowScriptsToggle height={360} />
        </div>
      ) : (
        <pre className="g8-source">{code}</pre>
      )}

      {showInfo && (
        <div className="g8-info" aria-label="Güvenlik ve içerik bilgisi">
          <p className="g8-info-row g8-info-net">
            <WifiOff size={14} aria-hidden="true" />
            <span>
              Ağ her zaman kapalıdır: önizleme hiçbir dış adrese bağlanamaz (CSP <code>connect-src 'none'</code>) ve bir alan adına izin verilemez. Betik, form ve canvas
              yalnızca betik iznini verdiğinizde çalışır; depolama (çerez, localStorage) da kapalıdır.
            </span>
          </p>
          {analysis.usesNetworkApi && <p className="g8-info-row">Betik ağ API'si (fetch/XHR/WebSocket) içeriyor; izin verseniz bile istekler engellenir.</p>}
          {analysis.blockedRefs.length > 0 && (
            <div className="g8-info-row">
              <span className="g8-info-label">Engellenen dış/kaynak istekleri:</span>
              <ul className="g8-info-list">
                {analysis.blockedRefs.slice(0, 12).map((r) => (
                  <li key={`${r.cls}:${r.url}`}>
                    <code>{r.url}</code> <span className="g8-muted">({r.cls})</span>
                  </li>
                ))}
                {analysis.blockedRefs.length > 12 && <li className="g8-muted">… ve {analysis.blockedRefs.length - 12} tane daha</li>}
              </ul>
            </div>
          )}
          {analysis.usesExternalFont && <p className="g8-info-row">Dış yazı tipi isteği var; yalnızca gömülü/yerel fontlar kullanılır (dış font engellenir).</p>}
          {analysis.externalLinks.length > 0 && (
            <div className="g8-info-row">
              <span className="g8-info-label">Önizlemedeki dış bağlantılar (önizleme içinden açılamaz):</span>
              <ul className="g8-info-list">
                {analysis.externalLinks.slice(0, 12).map((href) => (
                  <li key={href}>
                    <code>{href}</code>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {analysis.dataUris.length > 0 && (
            <p className="g8-info-row">
              {analysis.dataUris.length} gömülü görsel (data:), toplam {formatBytes(analysis.dataUriTotalBytes)}
              {analysis.dataUriOverLimit ? ' — bazıları boyut eşiğini aşıyor.' : '.'}
            </p>
          )}
          {analysis.a11yIssues.length > 0 && (
            <div className="g8-info-row">
              <span className="g8-info-label">Erişilebilirlik uyarıları:</span>
              <ul className="g8-info-list">
                {analysis.a11yIssues.map((i) => (
                  <li key={i.message}>{i.message}</li>
                ))}
              </ul>
              <span className="g8-muted">Not: renk kontrastı statik olarak denetlenemez.</span>
            </div>
          )}
          {analysis.customElements.length > 0 && <p className="g8-info-row">Özel elemanlar: {analysis.customElements.join(', ')} (yalnızca betik izniyle tanımlanır).</p>}
        </div>
      )}

      {expanded && (
        <Modal onClose={() => setExpanded(false)} labelledBy={expandTitleId}>
          <div className="dialog-body g8-expand">
            <ModalHost titleId={expandTitleId} title="HTML önizleme" code={code} />
            <div className="dialog-actions">
              <span />
              <button type="button" className="btn" onClick={() => setExpanded(false)}>
                Kapat
              </button>
            </div>
          </div>
        </Modal>
      )}

      {saving && (
        <NameDialog
          title="HTML'i dosyaya kaydet"
          description="Çalışma dizinine kaydedilecek dosya adı (ör. onizleme.html). Hassas dosyalar reddedilir, dizin dışına çıkılamaz."
          initial=""
          submitLabel="Kaydet"
          onSubmit={saveToFile}
          onClose={() => setSaving(false)}
        />
      )}
    </div>
  )
}

registerRichBlock('html', HtmlBlock)
