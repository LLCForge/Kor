import { ExternalLink, FileText, Paperclip, Pencil } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { EditorId, FilePreview as Preview } from '../ipc-g4.ts'
import { api, errorMessage } from './api.ts'
import { CodeEditor } from './CodeEditor.tsx'
import { getDraft } from './g4-shared.ts'
import { Markdown } from './Markdown.tsx'
import { usePersistentChoice } from './persisted.ts'
import { usePopover } from './use-popover.ts'
import { VirtualLines } from './VirtualLines.tsx'
// [g21] belge ve zengin dosya önizleme (PDF sayfaları, DOCX, XLSX/CSV, EPUB, arşiv, 3D, yazı tipi, EXIF, fark)
import { GitCompare } from 'lucide-react'
import { ComparePanel as G21ComparePanel, G21ImageMeta, G21Pdf, g21Body, g21CanCompare, g21CanRender } from './g21/G21Preview.tsx'

export const previewKey = (path: string) => ['file', path, 'g4'] as const

const EDITOR_LABEL: Record<EditorId | 'system', string> = { code: 'VS Code', cursor: 'Cursor', windsurf: 'Windsurf', zed: 'Zed', system: 'Varsayılan uygulama' }

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}

function ExternalOpenMenu({ path, line }: { path: string; line: number | null }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const { data: editors } = useSWR(open ? 'g4:editors' : null, () => api.g4.detectEditors(), { revalidateOnFocus: false })
  const [notice, setNotice] = useState<string | null>(null)

  const openWith = async (editor: EditorId | 'system') => {
    setOpen(false)
    setNotice(null)
    try {
      const used = await api.g4.openInEditor(path, line, editor)
      if (used === 'folder') setNotice('Çalıştırılabilir dosya güvenlik için açılmadı; klasörde gösterildi.')
      else if (used === 'system' && editor !== 'system') setNotice(`${EDITOR_LABEL[editor]} bulunamadı; varsayılan uygulamayla açıldı.`)
    } catch (err) {
      setNotice(errorMessage(err))
    }
  }

  return (
    <div className="popover-anchor" ref={ref}>
      <button
        type="button"
        className="btn small"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Harici editörde aç"
      >
        <ExternalLink size={14} aria-hidden="true" /> Aç
      </button>
      {open && (
        <div className="menu g4-menu pop-in" role="menu">
          {editors === undefined && <p className="menu-note">Editörler aranıyor...</p>}
          {editors?.map((id) => (
            <button key={id} type="button" role="menuitem" className="menu-row" onClick={() => void openWith(id)}>
              {EDITOR_LABEL[id]} ile aç{line ? ` (satır ${line})` : ''}
            </button>
          ))}
          {editors?.length === 0 && <p className="menu-note">VS Code veya Cursor komutu bulunamadı.</p>}
          <button type="button" role="menuitem" className="menu-row" onClick={() => void openWith('system')}>
            {EDITOR_LABEL.system} ile aç
          </button>
        </div>
      )}
      {notice && <p className="g4-toast" role="status">{notice}</p>}
    </div>
  )
}

interface FilePreviewProps {
  path: string
  line: number | null
  lineSeq: number
  onAttach: (path: string) => void
}

export function FilePreview({ path, line, lineSeq, onAttach }: FilePreviewProps) {
  const { data, error, isLoading, mutate } = useSWR(previewKey(path), () => api.g4.preview(path), { revalidateOnFocus: false })
  // Kaydedilmemiş taslağı olan dosya doğrudan editörde açılır.
  const [editing, setEditing] = useState(() => getDraft(path) !== undefined)
  const [renderPref, setRenderPref] = usePersistentChoice('g4.previewMode', ['render', 'source'] as const, 'render')
  const ext = path.slice(path.lastIndexOf('.') + 1).toLowerCase()
  const isMarkdown = ['md', 'markdown', 'mdx'].includes(ext)
  const canRender = data?.kind === 'text' && (isMarkdown || data.imageDataUrl !== undefined || g21CanRender(path)) // [g21] csv/tsv, gltf
  const [g21Compare, setG21Compare] = useState(false) // [g21] kelime farkı paneli
  // Satıra gitme isteği kaynak görünümünde anlamlıdır.
  const mode = canRender && line === null ? renderPref : 'source'

  return (
    <div className="panel-body">
      <div className="panel-file-head">
        <code title={path}>{path}</code>
        <div className="g4-bar-actions">
          {canRender && !editing && (
            <div className="g4-toggle" role="group" aria-label="Görünüm">
              <button type="button" aria-pressed={mode === 'render'} onClick={() => setRenderPref('render')}>
                Görünüm
              </button>
              <button type="button" aria-pressed={mode === 'source'} onClick={() => setRenderPref('source')}>
                Kaynak
              </button>
            </div>
          )}
          {data?.kind === 'text' && !editing && (
            <button
              type="button"
              className="btn small"
              onClick={() => setEditing(true)}
              disabled={!data.editable}
              title={data.editable ? 'Dosyayı düzenle' : 'Dosya düzenlemek için çok büyük'}
            >
              <Pencil size={14} aria-hidden="true" /> Düzenle
            </button>
          )}
          {/* [g21] belge karşılaştırma (kelime farkı) */}
          {data && g21CanCompare(path, data.kind) && (
            <button type="button" className={`btn small${g21Compare ? ' active' : ''}`} onClick={() => setG21Compare((c) => !c)} aria-pressed={g21Compare} title="Başka bir belgeyle kelime bazında karşılaştır">
              <GitCompare size={14} aria-hidden="true" /> Karşılaştır
            </button>
          )}
          {data?.kind !== 'sensitive' && <ExternalOpenMenu path={path} line={line} />}
          {data?.kind !== 'sensitive' && (
            <button type="button" className="btn small" onClick={() => onAttach(path)} title="Sohbete ekle">
              <Paperclip size={14} aria-hidden="true" /> Ekle
            </button>
          )}
        </div>
      </div>
      {isLoading && <p className="muted pad">Yükleniyor...</p>}
      {error && <p className="error-text pad" role="alert">{errorMessage(error)}</p>}
      {g21Compare && <G21ComparePanel key={path} path={path} onClose={() => setG21Compare(false)} />}
      {data && <PreviewBody path={path} data={data} mode={mode} line={line} lineSeq={lineSeq} editing={editing} onEditDone={() => setEditing(false)} onRefresh={() => void mutate()} />}
    </div>
  )
}

interface PreviewBodyProps {
  path: string
  data: Preview
  mode: 'render' | 'source'
  line: number | null
  lineSeq: number
  editing: boolean
  onEditDone: () => void
  onRefresh: () => void
}

function PreviewBody({ path, data, mode, line, lineSeq, editing, onEditDone, onRefresh }: PreviewBodyProps) {
  const g21 = g21Body({ path, data, mode, editing }) // [g21] belge/zengin dosya görüntüleyicileri
  if (g21) return g21
  switch (data.kind) {
    case 'sensitive':
      return <p className="muted pad">Hassas dosya (anahtar, parola vb. içerebilir): önizleme güvenlik için kapalı.</p>
    case 'binary':
      return <p className="muted pad">İkili dosya önizlenemez ({formatBytes(data.size)}).</p>
    case 'tooLarge':
      return (
        <p className="muted pad">
          Dosya önizleme için çok büyük ({formatBytes(data.size)}, sınır {formatBytes(data.limit)}). Harici editörde açabilirsin.
        </p>
      )
    case 'image':
      return (
        <div className="g4-image-wrap">
          <img className="g4-image" src={data.dataUrl} alt={`${path} önizlemesi`} />
          <p className="g4-meta">{formatBytes(data.size)}</p>
          <G21ImageMeta key={path} path={path} /> {/* [g21] EXIF/meta paneli (konum varsayılan gizli) */}
        </div>
      )
    case 'pdf':
      return (
        // [g21] sayfa görüntüleyici; pdf.js kullanılamazsa aşağıdaki metin görünümü yedek olarak kalır
        <G21Pdf key={path} path={path} fallback={
        <div className="g4-pdf">
          <p className="g4-meta">
            <FileText size={14} aria-hidden="true" /> PDF · {data.pages === null ? 'sayfa sayısı okunamadı' : `${data.pages} sayfa`} · {formatBytes(data.size)}
          </p>
          {data.text ? (
            <VirtualLines text={data.truncated ? `${data.text}\n[metin kesildi]` : data.text} label={`${path} metni`} />
          ) : (
            <p className="muted pad">Metin çıkarılamadı (taranmış belge veya desteklenmeyen yazı tipi kodlaması olabilir).</p>
          )}
        </div>
        } />
      )
    case 'text':
      if (editing) {
        return <CodeEditor key={path} path={path} content={data.content} mtimeMs={data.mtimeMs} line={line} lineSeq={lineSeq} onRefresh={onRefresh} onClose={onEditDone} />
      }
      if (mode === 'render' && data.imageDataUrl) {
        return (
          <div className="g4-image-wrap">
            <img className="g4-image" src={data.imageDataUrl} alt={`${path} önizlemesi`} />
          </div>
        )
      }
      if (mode === 'render') {
        return (
          <div className="g4-markdown prose">
            <Markdown text={data.content} />
          </div>
        )
      }
      return <VirtualLines text={data.content} highlightLine={line} scrollKey={lineSeq} label={`${path} içeriği`} />
  }
}
