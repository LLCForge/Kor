// Üretilen görsel ve videolar: generate_image / generate_video sonuçlarındaki `saved: <yol>` satırları.
// Dosyalar Tauri asset protokolüyle diskten akış olarak gösterilir (video ileri sarılabilir); erişim kapsamı
// yalnızca projedeki kor-media/ klasörüdür (src-tauri: allow_media).
import { convertFileSrc } from '@tauri-apps/api/core'
import type { ChatItem } from './chat-state.ts'

type ToolItem = Extract<ChatItem, { kind: 'tool' }>

// §20: Venice, fal ve Hugging Face medya araçları (ADR 0011, 0060).
const MEDIA_TOOLS = new Set(['generate_image', 'generate_video', 'image_generate', 'image_edit', 'video_generate', 'text_to_speech', 'fal_run'])
const VIDEO = /\.(mp4|webm|mov)$/i
const AUDIO = /\.(mp3|wav|ogg|flac|m4a)$/i

/** Başarılı medya araçlarının kaydettiği dosyalar, sırayla. */
export function mediaFiles(tools: readonly ToolItem[]): string[] {
  const files: string[] = []
  for (const t of tools) {
    if (!MEDIA_TOOLS.has(t.name) || !t.result) continue
    for (const m of t.result.matchAll(/^saved: (.+)$/gm)) {
      const path = m[1]?.trim()
      if (path) files.push(path)
    }
  }
  return files
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path
}

export function MediaStrip({ tools }: { tools: readonly ToolItem[] }) {
  const files = mediaFiles(tools)
  if (files.length === 0) return null
  return (
    <div className="media-strip">
      {files.map((path) =>
        VIDEO.test(path) ? (
          <video key={path} className="media-item" src={convertFileSrc(path)} controls preload="metadata" aria-label={fileName(path)} />
        ) : AUDIO.test(path) ? (
          <audio key={path} src={convertFileSrc(path)} controls preload="metadata" aria-label={fileName(path)} />
        ) : (
          <img key={path} className="media-item" src={convertFileSrc(path)} alt={fileName(path)} title={path} loading="lazy" />
        ),
      )}
    </div>
  )
}
