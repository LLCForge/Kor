/**
 * Windows'un uzun yol öneki (`\\?\C:\…`, `\\?\UNC\sunucu\pay`) yalnızca API içindir; arayüzde gösterilmez.
 * Yol değeri değişmez (oturumlar ve araçlar kanonik yolla eşleşir), yalnızca gösterilen metin sadeleşir.
 */
export function displayPath(path: string): string {
  if (path.startsWith('\\\\?\\UNC\\')) return `\\\\${path.slice(8)}`
  return path.startsWith('\\\\?\\') ? path.slice(4) : path
}
