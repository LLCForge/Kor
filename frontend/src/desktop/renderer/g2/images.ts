// Yapıştırılan/sürüklenen görselleri okur. Asıl doğrulama (boyut, sihirli bayt) ana süreçte yapılır;
// buradaki kontroller yalnızca kullanıcıya erken geri bildirim içindir.

export interface PendingImage {
  id: string
  name: string
  dataUrl: string
  size: number
}

export const MAX_IMAGES = 8
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024
export const IMAGE_TYPES: readonly string[] = ['image/png', 'image/jpeg', 'image/webp', 'image/gif']

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => (typeof reader.result === 'string' ? resolve(reader.result) : reject(new Error('okunamadı')))
    reader.onerror = () => reject(reader.error ?? new Error('okunamadı'))
    reader.readAsDataURL(file)
  })
}

export async function readImageFiles(files: readonly File[], room: number): Promise<{ images: PendingImage[]; errors: string[] }> {
  const images: PendingImage[] = []
  const errors: string[] = []
  for (const file of files) {
    const name = file.name || 'yapıştırılan-görsel'
    if (!IMAGE_TYPES.includes(file.type)) errors.push(`${name}: yalnızca PNG, JPEG, WebP ve GIF desteklenir.`)
    else if (file.size > MAX_IMAGE_BYTES) errors.push(`${name}: görsel 5 MB sınırını aşıyor.`)
    else if (images.length >= room) errors.push(`En fazla ${MAX_IMAGES} görsel eklenebilir.`)
    else {
      try {
        images.push({ id: crypto.randomUUID(), name, dataUrl: await readAsDataUrl(file), size: file.size })
      } catch (err) {
        console.error('Görsel okunamadı:', err)
        errors.push(`${name}: görsel okunamadı.`)
      }
    }
  }
  return { images, errors: [...new Set(errors)] }
}
