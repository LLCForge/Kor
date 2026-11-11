// [g21] Görsel EXIF/meta paneli (#181, konum varsayılan gizli), 3D model (#182), yazı tipi (#183) ve arşiv
//      listeleme (#184, çıkarmadan). Hepsi baytları ana sürecin hapsedilmiş okuma API'sinden alır; ağ yok.
import { Box, File, FileWarning, Folder, MapPin, RotateCcw, RotateCw } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import useSWR from 'swr'
import { api } from '../api.ts'
import { formatBytes } from '../FilePreview.tsx'
import { loadRichBundle } from '../rich-lazy.ts'
import { DocBar, Failure, Loading, useDocQuery } from './DocChrome.tsx'
import { readFontInfo } from './font-info.ts'
import { externalUris, packGlb, parseModel } from './gltf-pack.ts'
import { readImageMeta, type MetaEntry } from './image-meta.ts'
import type { G21ThreeApi, ModelHandle } from './three-entry.ts'
import { entryWarnings, listZip, resolveRelative, zipTree, type ZipTreeNode } from './zip.ts'
import { uiLocale } from '../ui-locale.ts'

export function ImageMetaPanel({ path }: { path: string }) {
  const { data, error } = useDocQuery(['g21', 'image-meta', path], async () => readImageMeta(await api.g21ReadDoc(path)))
  const [showLocation, setShowLocation] = useState(false)
  if (error) return <Failure error={error} />
  if (!data) return <Loading label="Meta veri okunuyor..." />
  const visible = data.entries.filter((e) => showLocation || !e.sensitive)
  const groups = new Map<MetaEntry['group'], MetaEntry[]>()
  for (const e of visible) groups.set(e.group, [...(groups.get(e.group) ?? []), e])
  return (
    <section className="g21-meta" aria-label="Görsel bilgileri">
      <dl className="g21-dl">
        <dt>Biçim</dt>
        <dd>{data.format}</dd>
        {data.width !== null && data.height !== null && (
          <>
            <dt>Boyut</dt>
            <dd>
              {data.width} × {data.height} px
            </dd>
          </>
        )}
        {[...groups].flatMap(([group, entries]) =>
          entries.map((e, i) => (
            <div key={`${group}-${e.label}-${i}`} className={e.sensitive ? 'g21-sensitive' : undefined}>
              <dt>
                {i === 0 && <span className="g21-group">{group} · </span>}
                {e.label}
              </dt>
              <dd>{e.value}</dd>
            </div>
          )),
        )}
      </dl>
      {data.hasLocation && (
        <button type="button" className="btn small" onClick={() => setShowLocation((s) => !s)} aria-pressed={showLocation}>
          <MapPin size={14} aria-hidden="true" /> {showLocation ? 'Konum verisini gizle' : 'Konum verisi gizli: göster'}
        </button>
      )}
      {data.entries.length === 0 && <p className="g21-muted">Gömülü meta veri (EXIF/metin) yok.</p>}
    </section>
  )
}

const loadThree = () => loadRichBundle<G21ThreeApi>('./g21-three.js', '__g21Three')

/** Modeli ve yerel dış kaynaklarını okuyup tek GLB'ye paketler. */
async function loadModel(path: string): Promise<Uint8Array> {
  const model = parseModel(await api.g21ReadDoc(path))
  const external = new Map<string, Uint8Array>()
  for (const uri of externalUris(model.json)) external.set(uri, await api.g21ReadDoc(resolveRelative(path, uri)))
  return packGlb(model, external)
}

export function ModelViewer({ path }: { path: string }) {
  const glb = useDocQuery(['g21', 'model', path], () => loadModel(path))
  const lib = useSWR('g21:lib:three', loadThree, { revalidateOnFocus: false, shouldRetryOnError: false })
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const handleRef = useRef<ModelHandle | null>(null)
  const [stats, setStats] = useState<ModelHandle['stats'] | null>(null)
  const [mountError, setMountError] = useState<unknown>(null)
  const [spin, setSpin] = useState(false)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !glb.data || !lib.data) return
    let disposed = false
    setMountError(null)
    const data = glb.data
    lib.data
      .mountModel(canvas, data.slice().buffer)
      .then((handle) => {
        if (disposed) return handle.dispose()
        handleRef.current = handle
        setStats(handle.stats)
      })
      .catch((err: unknown) => {
        console.error('3D model gösterilemedi:', err)
        if (!disposed) setMountError(new Error('Model gösterilemedi (desteklenmeyen uzantı, ör. Draco/KTX2, ya da bozuk dosya).'))
      })
    return () => {
      disposed = true
      handleRef.current?.dispose()
      handleRef.current = null
    }
  }, [glb.data, lib.data])

  const error = glb.error ?? lib.error ?? mountError
  return (
    <div className="g21-viewer">
      <DocBar>
        {stats && (
          <span className="g21-muted">
            <Box size={13} aria-hidden="true" /> {stats.meshes} ağ · {stats.triangles.toLocaleString(uiLocale())} üçgen · {stats.materials} malzeme
            {stats.animations > 0 ? ` · ${stats.animations} animasyon (oynatılmaz)` : ''}
          </span>
        )}
        <button type="button" className="action-btn" onClick={() => handleRef.current?.resetView()} disabled={!stats} title="Görünümü sıfırla" aria-label="Görünümü sıfırla">
          <RotateCcw size={15} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={`action-btn${spin ? ' active' : ''}`}
          onClick={() => {
            handleRef.current?.setAutoRotate(!spin)
            setSpin(!spin)
          }}
          disabled={!stats}
          aria-pressed={spin}
          title="Otomatik döndür"
          aria-label="Otomatik döndür"
        >
          <RotateCw size={15} aria-hidden="true" />
        </button>
      </DocBar>
      {error ? <Failure error={error} /> : !stats && <Loading label="Model hazırlanıyor..." />}
      <canvas ref={canvasRef} className="g21-model" aria-label={`${path} 3B model önizlemesi; sürükleyerek döndür, tekerlekle yakınlaştır`} />
    </div>
  )
}

const SAMPLE = 'Pijamalı hasta yağız şoföre çabucak güvendi. 0123456789'
const GLYPHS = 'ABCÇDEFGĞHIİJKLMNOÖPRSŞTUÜVYZ abcçdefgğhıijklmnoöprsştuüvyz 0123456789 .,;:!?()[]{}&@#%*+-=/'
let fontSeq = 0

export function FontViewer({ path }: { path: string }) {
  const { data, error } = useDocQuery(['g21', 'font', path], async () => {
    const bytes = await api.g21ReadDoc(path)
    return { bytes, info: await readFontInfo(bytes) }
  })
  const family = useMemo(() => `g21-font-${++fontSeq}`, [])
  const [ready, setReady] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [sample, setSample] = useState(SAMPLE)

  // FontFace ikili veriden yüklenir (URL/ağ yok); Chromium yazı tipini OTS ile doğrular.
  useEffect(() => {
    if (!data) return
    const face = new FontFace(family, data.bytes.slice())
    let active = true
    face.load().then(
      () => {
        if (!active) return
        document.fonts.add(face)
        setReady(true)
      },
      (err: unknown) => {
        console.warn('Yazı tipi yüklenemedi:', err)
        if (active) setLoadError('Yazı tipi tarayıcı tarafından reddedildi (bozuk veya desteklenmeyen dosya).')
      },
    )
    return () => {
      active = false
      document.fonts.delete(face)
    }
  }, [data, family])

  if (error) return <Failure error={error} />
  if (!data) return <Loading label="Yazı tipi okunuyor..." />
  const { info } = data
  const rows: Array<[string, string | number | undefined]> = [
    ['Biçim', info.format],
    ['Aile', info.family],
    ['Stil', info.subfamily],
    ['Tam ad', info.fullName],
    ['Sürüm', info.version],
    ['Tasarımcı', info.designer],
    ['Glif sayısı', info.glyphs],
    ['Lisans', info.license],
  ]
  return (
    <div className="g21-viewer">
      <div className="g21-scroll">
        <dl className="g21-dl">
          {rows
            .filter(([, v]) => v !== undefined && v !== '')
            .map(([k, v]) => (
              <div key={k}>
                <dt>{k}</dt>
                <dd>{v}</dd>
              </div>
            ))}
        </dl>
        {loadError && <p className="error-text pad">{loadError}</p>}
        {ready && (
          <div className="g21-font" style={{ fontFamily: `"${family}"` }}>
            <input className="g8-input g21-font-input" value={sample} onChange={(e) => setSample(e.target.value.slice(0, 200))} aria-label="Örnek metin" style={{ fontFamily: 'var(--font-sans)' }} />
            {[14, 20, 32, 48].map((size) => (
              <p key={size} style={{ fontSize: size }}>
                <small className="g21-muted">{size}px</small> {sample}
              </p>
            ))}
            <p className="g21-glyphs">{GLYPHS}</p>
          </div>
        )}
      </div>
    </div>
  )
}

function TreeNode({ node, depth }: { node: ZipTreeNode; depth: number }) {
  // Kapalı klasörün çocukları çizilmez: on binlerce girişli arşivde DOM küçük kalır.
  const [open, setOpen] = useState(depth < 1)
  if (!node.dir) {
    return (
      <li className={node.warnings.length ? 'g21-warn' : undefined}>
        {node.warnings.length ? <FileWarning size={13} aria-hidden="true" /> : <File size={13} aria-hidden="true" />}
        <span className="g21-name">{node.name}</span>
        <span className="g21-muted">{formatBytes(node.size)}</span>
        {node.warnings.length > 0 && <span className="g21-flag">{node.warnings.join(', ')}</span>}
      </li>
    )
  }
  return (
    <li>
      <details open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
        <summary>
          <Folder size={13} aria-hidden="true" /> <span className="g21-name">{node.name}</span>
          <span className="g21-muted">
            {node.children.length} öğe · {formatBytes(node.size)}
          </span>
          {node.warnings.length > 0 && <span className="g21-flag">{node.warnings.join(', ')}</span>}
        </summary>
        {open && (
          <ul>
            {node.children.map((c) => (
              <TreeNode key={c.path} node={c} depth={depth + 1} />
            ))}
          </ul>
        )}
      </details>
    </li>
  )
}

export function ArchiveViewer({ path }: { path: string }) {
  const { data, error } = useDocQuery(['g21', 'zip', path], async () => {
    const entries = listZip(await api.g21ReadDoc(path))
    return { entries, tree: zipTree(entries) }
  })
  if (error) return <Failure error={error} />
  if (!data) return <Loading label="Arşiv listeleniyor..." />
  const files = data.entries.filter((e) => !e.dir)
  const flagged = files.filter((e) => entryWarnings(e).length > 0).length
  const ratio = data.tree.compressedSize > 0 ? data.tree.size / data.tree.compressedSize : 1
  return (
    <div className="g21-viewer">
      <DocBar>
        <span className="g21-muted">
          {files.length} dosya · açılmış {formatBytes(data.tree.size)} · sıkıştırılmış {formatBytes(data.tree.compressedSize)} · oran {ratio.toFixed(1)}×
          {flagged > 0 ? ` · ${flagged} riskli giriş` : ''}
        </span>
      </DocBar>
      <p className="g21-muted g21-pad">Yalnızca liste: arşiv çıkarılmaz, içindeki dosyalar çalıştırılmaz.</p>
      <div className="g21-scroll">
        <ul className="g21-tree">
          {data.tree.children.map((c) => (
            <TreeNode key={c.path} node={c} depth={0} />
          ))}
        </ul>
      </div>
    </div>
  )
}
