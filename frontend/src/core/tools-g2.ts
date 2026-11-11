// Bu modül yalnızca tools.ts üzerinden yüklenmeli (döngüsel içe aktarma: defineTool tools.ts'te tanımlı).
import { z } from 'zod'
import { readImageFile } from './attachments.ts'
import { isSensitivePath } from './permissions.ts'
import { defineTool, resolveInWorkspace, type Tool } from './tools.ts'
import { registerToolImage } from './user-queue.ts'

export const g2Tools: Tool[] = [
  defineTool({
    name: 'read_image',
    description:
      'Çalışma dizinindeki bir görseli (PNG, JPEG, WebP, GIF; en fazla 5 MB) okur; görsel bir sonraki adımda sana görsel olarak eklenir. ' +
      'Ekran görüntüsü, diyagram veya arayüz tasarımını incelemek için kullan. Yalnızca görsel girişi destekleyen modellerde işe yarar.',
    schema: z.object({ path: z.string().min(1).max(1024).describe('Çalışma dizinine göre görsel dosyasının yolu') }),
    kind: 'read',
    summarize: (a) => a.path,
    subject: (a) => a.path,
    run: async (a, ctx) => {
      const abs = resolveInWorkspace(ctx.root, a.path)
      if (isSensitivePath(abs)) throw new Error(`Hassas dosyaya erişim engellendi: ${a.path}`)
      const image = await readImageFile(abs, a.path)
      const token = registerToolImage(a.path, image.dataUrl)
      return `Görsel okundu: ${a.path} (${image.mime}, ${Math.ceil(image.size / 1024)} KB). Görsel bir sonraki mesajda sana eklenecek. ${token}`
    },
  }),
]
