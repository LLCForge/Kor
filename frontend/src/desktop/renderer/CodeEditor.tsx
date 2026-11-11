import type { Text } from '@codemirror/state'
import type { EditorView } from '@codemirror/view'
import { Save, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { confirmAction } from './ConfirmDialog.tsx'
import { api, errorMessage } from './api.ts'
import type { CodeMirrorApi } from './codemirror-entry.ts'
import type { SaveResult } from '../ipc-g4.ts'
import { clearDraft, getDraft, setDraft } from './g4-shared.ts'
import { loadRichBundle } from './rich-lazy.ts'

// [g16] CodeMirror ilk açılış paketinde değil (242): editör ilk kez açılınca codemirror.js tembel yüklenir.
// Diller, tema ve uzantılar codemirror-entry.ts içindedir; bu dosya CodeMirror'dan yalnızca tip alır.
const readyCodeMirror = () => (window as { __g16CodeMirror?: CodeMirrorApi }).__g16CodeMirror

interface CodeEditorProps {
  path: string
  /** Diskteki içerik ve sürümü (önizleme verisi) */
  content: string
  mtimeMs: number
  line: number | null
  /** Aynı dosyada yeniden satıra gitme isteği */
  lineSeq: number
  /** Önizleme verisini diskten yeniler (kayıttan / çakışmadan sonra) */
  onRefresh: () => void
  onClose: () => void
  /** Çalışma klasörü dışındaki dosyalar için kayıt (ör. Hafıza paneli); yoksa g4.writeFile */
  write?: (content: string, expectedMtimeMs: number | null) => Promise<SaveResult>
}

function goToLine(cm: CodeMirrorApi, view: EditorView, line: number | null): void {
  if (line === null || line < 1 || line > view.state.doc.lines) return
  const pos = view.state.doc.line(line).from
  view.dispatch({ selection: { anchor: pos }, effects: cm.scrollIntoView(pos, { y: 'center' }) })
}

export function CodeEditor(props: CodeEditorProps) {
  const [cm, setCm] = useState(readyCodeMirror)
  const [loadError, setLoadError] = useState<string | null>(null)
  useEffect(() => {
    if (cm) return
    let cancelled = false
    loadRichBundle<CodeMirrorApi>('./codemirror.js', '__g16CodeMirror')
      .then((loaded) => {
        if (!cancelled) setCm(loaded)
      })
      .catch((err: unknown) => {
        console.error('Düzenleyici yüklenemedi:', err)
        if (!cancelled) setLoadError(errorMessage(err))
      })
    return () => {
      cancelled = true
    }
  }, [cm])
  if (loadError) return <p className="error-text g4-editor-error" role="alert">Düzenleyici yüklenemedi: {loadError}</p>
  if (!cm) return <p className="muted g4-editor-error" role="status">Düzenleyici yükleniyor...</p>
  return <CodeEditorView cm={cm} {...props} />
}

function CodeEditorView({ cm, path, content, mtimeMs, line, lineSeq, onRefresh, onClose, write }: CodeEditorProps & { cm: CodeMirrorApi }) {
  const host = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const savedDoc = useRef<Text | null>(null)
  const baseMtime = useRef(getDraft(path)?.baseMtimeMs ?? mtimeMs)
  const [dirty, setDirty] = useState(() => getDraft(path) !== undefined)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [conflict, setConflict] = useState(false)
  const [diskChanged, setDiskChanged] = useState(() => {
    const draft = getDraft(path)
    return draft !== undefined && draft.baseMtimeMs !== mtimeMs
  })

  const save = useCallback(
    async (force: boolean) => {
      const view = viewRef.current
      if (!view || saving) return
      const doc = view.state.doc
      setSaving(true)
      setError(null)
      try {
        // sliceDoc dosyanın satır sonu biçimini (lineSeparator) korur.
        const expected = force ? null : baseMtime.current
        const result = write ? await write(view.state.sliceDoc(), expected) : await api.g4.writeFile(path, view.state.sliceDoc(), expected)
        if (!result.ok) {
          setConflict(true)
          onRefresh()
          return
        }
        baseMtime.current = result.mtimeMs
        savedDoc.current = doc
        setConflict(false)
        setDiskChanged(false)
        const stillDirty = !view.state.doc.eq(doc)
        setDirty(stillDirty)
        if (stillDirty) setDraft(path, { doc: view.state.doc, baseMtimeMs: result.mtimeMs })
        else clearDraft(path)
        onRefresh()
      } catch (err) {
        setError(errorMessage(err))
      } finally {
        setSaving(false)
      }
    },
    [path, saving, onRefresh, write],
  )
  const saveRef = useRef(save)
  saveRef.current = save

  // Editör yalnızca dosya değişince kurulur (içerik ve satır kurulumda bir kez okunur); disk güncellemeleri aşağıdaki etkide.
  useEffect(() => {
    if (!host.current) return
    const shadow = host.current.shadowRoot ?? host.current.attachShadow({ mode: 'open' })
    const container = document.createElement('div')
    container.style.height = '100%'
    shadow.appendChild(container)
    const { view, diskDoc } = cm.createEditor({
      parent: container,
      root: shadow,
      path,
      content,
      draft: getDraft(path)?.doc,
      onSave: () => void saveRef.current(false),
      onDocChanged: (doc) => {
        if (!savedDoc.current) return
        const isDirty = !doc.eq(savedDoc.current)
        setDirty(isDirty)
        if (isDirty) setDraft(path, { doc, baseMtimeMs: baseMtime.current })
        else clearDraft(path)
      },
    })
    savedDoc.current = diskDoc
    viewRef.current = view
    goToLine(cm, view, line)
    view.focus()
    return () => {
      view.destroy()
      container.remove()
      viewRef.current = null
    }
  }, [path])

  // Editör açıkken gelen satıra git istekleri (ör. tanılama listesinden). İlk satır kurulumda uygulanır.
  const mountedSeq = useRef(lineSeq)
  useEffect(() => {
    const view = viewRef.current
    if (!view || lineSeq === mountedSeq.current) return
    goToLine(cm, view, line)
    view.focus()
  }, [line, lineSeq])

  // Dosya diskte değişti (izleyici veya kayıt sonrası yenileme): temizse sessizce yüklenir, kirliyse uyarılır.
  useEffect(() => {
    const view = viewRef.current
    if (!view || mtimeMs === baseMtime.current) return
    if (dirty) {
      setDiskChanged(true)
      return
    }
    reload(view)
  }, [mtimeMs, content])

  const reload = (view: EditorView) => {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: content } })
    savedDoc.current = view.state.doc
    baseMtime.current = mtimeMs
    setDirty(false)
    setDiskChanged(false)
    setConflict(false)
    clearDraft(path)
  }

  const close = async () => {
    if (
      dirty &&
      !(await confirmAction({
        title: 'Editör kapatılsın mı?',
        message: 'Kaydedilmemiş değişiklikler kaybolacak.',
        confirmLabel: 'Kaydetmeden kapat',
        danger: true,
      }))
    ) {
      return
    }
    clearDraft(path)
    onClose()
  }

  return (
    <div className="g4-editor">
      <div className="g4-editor-bar">
        <span className={`g4-dirty${dirty ? ' on' : ''}`} role="status">
          {dirty ? 'Kaydedilmemiş değişiklikler' : 'Kaydedildi'}
        </span>
        <div className="g4-bar-actions">
          <button type="button" className="btn small primary" onClick={() => void save(false)} disabled={saving || !dirty} title="Kaydet (Ctrl+S)">
            <Save size={14} aria-hidden="true" /> {saving ? 'Kaydediliyor...' : 'Kaydet'}
          </button>
          <button type="button" className="btn small" onClick={() => void close()} title="Editörü kapat">
            <X size={14} aria-hidden="true" /> Kapat
          </button>
        </div>
      </div>
      {(diskChanged || conflict) && (
        <div className="g4-banner" role="alert">
          <span>
            {conflict
              ? 'Dosya sen düzenlerken diskte değişti; kaydetmek o değişikliklerin üzerine yazar.'
              : 'Dosya diskte değişti. Kaydedersen diskteki değişikliklerin üzerine yazılır.'}
          </span>
          <span className="g4-bar-actions">
            {conflict && (
              <button type="button" className="btn small" onClick={() => void save(true)} disabled={saving}>
                Yine de kaydet
              </button>
            )}
            <button
              type="button"
              className="btn small"
              onClick={() => {
                void confirmAction({
                  title: 'Diskteki sürüm yüklensin mi?',
                  message: 'Editördeki düzenlemelerin atılacak ve dosyanın diskteki hali yüklenecek.',
                  confirmLabel: 'Diskten yükle',
                  danger: true,
                }).then((ok) => {
                  const view = viewRef.current
                  if (ok && view) reload(view)
                })
              }}
            >
              Diskten yükle
            </button>
          </span>
        </div>
      )}
      {error && <p className="error-text g4-editor-error" role="alert">{error}</p>}
      <div ref={host} className="g4-editor-host" />
    </div>
  )
}
