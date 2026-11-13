// Galeri (§20.3, §20.4): Kor'un ürettiği görsel/video/ses, masaüstüyle aynı `kor-media/` klasöründen, mevcut
// köprü bağlantısıyla. Küçük resimler çekirdekte küçültülür; açma, kopyalama ve indirme tam dosyayı parça parça alır.
import { useCallback, useEffect, useRef, useState } from 'react'
import { Copy, Download, Film, Music, RefreshCw, X } from 'lucide-react'
import { client } from './bus.ts'
import { downloadName, fetchMedia, type GalleryItem } from './media.ts'
import { t } from './i18n.ts'

const PAGE = 60

export function Gallery() {
  const [items, setItems] = useState<GalleryItem[] | null>(null)
  const [error, setError] = useState('')
  const [shown, setShown] = useState(PAGE)
  const [thumbs, setThumbs] = useState<Record<string, string>>({})
  const [open, setOpen] = useState<GalleryItem | null>(null)
  const close = useCallback(() => setOpen(null), [])

  const load = () => {
    setError('')
    client<GalleryItem[]>('gallery').then(setItems, () => {
      setItems([])
      setError(t('Medya alınamadı.'))
    })
  }
  useEffect(load, [])

  // Küçük resimler sırayla (köprüyü boğmadan) yalnız görünen sayfa için istenir.
  useEffect(() => {
    let live = true
    const want = (items ?? []).slice(0, shown).filter((i) => i.kind === 'image' && !(i.id in thumbs))
    void (async () => {
      for (const i of want) {
        const r = await client<{ data: string } | null>('galleryThumb', { id: i.id }).catch(() => null)
        if (!live) return
        setThumbs((all) => ({ ...all, [i.id]: r?.data ?? '' }))
      }
    })()
    return () => {
      live = false
    }
    // thumbs bilerek yok: her yeni küçük resim döngüyü yeniden başlatmasın.
  }, [items, shown])

  return (
    <section className="gallery-view" aria-label={t('Galeri')}>
      <h2>
        <span className="grow">{t('Galeri')}</span>
        <button type="button" className="icon" aria-label={t('Yenile')} title={t('Yenile')} onClick={load}>
          <RefreshCw size={14} />
        </button>
      </h2>
      {error ? <p className="err" role="alert">{error}</p> : null}
      {items === null ? <p className="note">{t('Yükleniyor…')}</p> : null}
      {items?.length === 0 && !error ? <p className="note">{t('Galeri boş: Kor ile üretilen görsel, video ve sesler burada görünür.')}</p> : null}
      <ul className="gallery">
        {(items ?? []).slice(0, shown).map((i) => (
          <li key={i.id}>
            <button type="button" className="tile" onClick={() => setOpen(i)} aria-label={`${t('Aç')}: ${i.prompt || i.id}`} title={i.prompt || i.id}>
              {i.kind === 'image' && thumbs[i.id] ? (
                <img src={thumbs[i.id]} alt="" />
              ) : i.kind === 'video' ? (
                <Film size={24} aria-hidden="true" />
              ) : i.kind === 'audio' ? (
                <Music size={24} aria-hidden="true" />
              ) : (
                <span className="note">{i.kind === 'image' ? t('Yükleniyor…') : i.id}</span>
              )}
            </button>
          </li>
        ))}
      </ul>
      {(items?.length ?? 0) > shown ? (
        <button type="button" onClick={() => setShown((n) => n + PAGE)}>
          {t('Daha fazla')}
        </button>
      ) : null}
      {open ? <Viewer item={open} onClose={close} /> : null}
    </section>
  )
}

/** Tam boy önizleme: dosya bir kez alınır, kapanınca blob URL bırakılır. */
function Viewer({ item, onClose }: { item: GalleryItem; onClose: () => void }) {
  const [blob, setBlob] = useState<Blob | null>(null)
  const [url, setUrl] = useState('')
  const [msg, setMsg] = useState('')
  const closeRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    let live = true
    let made = ''
    closeRef.current?.focus()
    fetchMedia((op, body) => client(op, body), item.id).then(
      (b) => {
        if (!live) return
        made = URL.createObjectURL(b)
        setBlob(b)
        setUrl(made)
      },
      () => live && setMsg(t('Medya alınamadı.')),
    )
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      live = false
      window.removeEventListener('keydown', onKey)
      if (made) URL.revokeObjectURL(made)
    }
  }, [item.id, onClose])

  // Pano yalnız PNG görsel kabul eder: diğer biçimler tuvalde PNG'ye çevrilir.
  const copy = async () => {
    if (!blob) return
    try {
      const png = blob.type === 'image/png' ? blob : await toPng(blob)
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })])
      setMsg(t('Kopyalandı'))
    } catch {
      setMsg(t('Kopyalanamadı.'))
    }
  }

  return (
    <div className="viewer" role="dialog" aria-modal="true" aria-label={item.prompt || item.id}>
      <div className="row">
        <span className="grow note" translate="no">{item.model}</span>
        {item.kind === 'image' ? (
          <button type="button" onClick={() => void copy()} disabled={!blob}>
            <Copy size={14} aria-hidden="true" /> {t('Kopyala')}
          </button>
        ) : null}
        <a className={`btn${url ? '' : ' disabled'}`} href={url || undefined} download={downloadName(item.id)} aria-disabled={!url}>
          <Download size={14} aria-hidden="true" /> {t('İndir')}
        </a>
        <button ref={closeRef} type="button" className="icon" aria-label={t('Kapat')} title={t('Kapat')} onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      {msg ? <p className="note" role="status">{msg}</p> : null}
      <div className="viewer-body">
        {!url ? (
          <p className="note">{t('Yükleniyor…')}</p>
        ) : item.kind === 'image' ? (
          <img src={url} alt={item.prompt || item.id} />
        ) : item.kind === 'video' ? (
          <video src={url} controls />
        ) : item.kind === 'audio' ? (
          <audio src={url} controls />
        ) : (
          <p className="note" translate="no">{item.id}</p>
        )}
      </div>
      {item.prompt ? <p className="prompt" translate="no">{item.prompt}</p> : null}
    </div>
  )
}

async function toPng(blob: Blob): Promise<Blob> {
  const bmp = await createImageBitmap(blob)
  const canvas = new OffscreenCanvas(bmp.width, bmp.height)
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('canvas')
  ctx.drawImage(bmp, 0, 0)
  bmp.close()
  return canvas.convertToBlob({ type: 'image/png' })
}
