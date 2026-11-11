// Slash komut gövdesi genişletme (saf; Node/DOM yok — hem çekirdek hem arayüz kullanır).

/** $ARGUMENTS tüm argüman metnine, $1..$9 boşlukla ayrılmış konumsal argümanlara (yoksa boş) genişler. */
export function expandCommand(body: string, args: string): string {
  const trimmed = args.trim()
  const parts = trimmed ? trimmed.split(/\s+/) : []
  return body.replace(/\$ARGUMENTS\b/g, trimmed).replace(/\$([1-9])/g, (_m, d: string) => parts[Number(d) - 1] ?? '')
}
