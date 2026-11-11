/**
 * SMIL animasyonlu SVG'nin durağan hali: <img> içindeki SMIL sayfa CSS'inden etkilenmez, bu yüzden hareket kapalıyken
 * (`prefers-reduced-motion`, Animasyonlar = Kapalı, düşük güç) `<animate*>` öğeleri çıkarılmış kopya çizilir (§16.2 kural 3).
 */
export function stillSvg(svg: string): string {
  return svg.replace(/<animate\w*\b[^>]*?(?:\/>|>[\s\S]*?<\/animate\w*>)/g, '')
}
