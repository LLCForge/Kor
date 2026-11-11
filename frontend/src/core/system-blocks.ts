/**
 * Sistem mesajındaki uygulama blokları (g11 hafıza/zekâ, g13 istem katmanları, ...).
 * Her blok kendi `<!-- id:begin -->` / `<!-- id:end -->` işaretleri arasında durur. Bir grup yalnızca kendi
 * bloğunu yerinde günceller: başka grupların bloklarını silemez, sıraları kaymaz. Değişmeyen blokta metin aynı
 * kalır ki istem önbelleği bozulmasın.
 */
const beginOf = (id: string) => `<!-- ${id}:begin -->`
const endOf = (id: string) => `<!-- ${id}:end -->`

function locate(system: string, id: string): { start: number; stop: number } | null {
  const start = system.indexOf(`\n\n${beginOf(id)}`)
  if (start === -1) return null
  const end = system.indexOf(endOf(id), start)
  // Kapanışı olmayan (bozuk) blok sona kadar sayılır: yeniden yazımda kendini onarır.
  return { start, stop: end === -1 ? system.length : end + endOf(id).length }
}

/** Bloğu kaldırır; öncesi ve sonrası (ör. alt ajan rol notu) korunur. */
export function stripSystemBlock(system: string, id: string): string {
  const at = locate(system, id)
  return at ? system.slice(0, at.start) + system.slice(at.stop) : system
}

/**
 * Otomatik bağlam: masaüstünde yan panelde açık dosya. Yalnızca dosya değişince metin değişir (her turda değil),
 * böylece istem önbelleği gereksiz yere bozulmaz. Yol JSON olarak yazılır: dosya adındaki satır sonu talimat sızdıramaz.
 */
export function withOpenFileBlock(system: string, relPath: string | null): string {
  return upsertSystemBlock(
    system,
    'open-file',
    relPath ? `# OPEN FILE\nThe user has ${JSON.stringify(relPath)} open in the side panel. It may or may not be related to the request.` : '',
  )
}

/** Bloğu yerinde günceller, yoksa sona ekler; boş metin bloğu kaldırır. */
export function upsertSystemBlock(system: string, id: string, text: string): string {
  const at = locate(system, id)
  const block = text ? `\n\n${beginOf(id)}\n${text}\n${endOf(id)}` : ''
  return at ? system.slice(0, at.start) + block + system.slice(at.stop) : system + block
}
