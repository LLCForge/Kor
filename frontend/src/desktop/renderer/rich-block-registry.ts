// [temel] Zengin blok kayıt defteri: kod bloğu dilini (```mermaid, ```chart, ```dot, ```csv, ```html ...)
//         bir React renderer bileşenine eşler. Markdown kod işleyicisinde TEK arama noktasıdır:
//         dil kayıtlıysa o bileşen çizilir, değilse normal kod bloğu. Gruplar registerRichBlock ile kaydolur.
import type { ComponentType } from 'react'

/** Her zengin blok bileşeni ham kod metnini ve dil etiketini alır. */
export interface RichBlockProps {
  code: string
  language: string
}

const registry = new Map<string, ComponentType<RichBlockProps>>()

/** Dil anahtarı küçük harfe indirilip boşlukları kırpılarak normalize edilir. */
function normalize(language: string): string {
  return language.trim().toLowerCase()
}

/** Bir dili (ör. 'mermaid') bir renderer bileşenine kaydeder. Son kayıt öncekini geçersiz kılar. */
export function registerRichBlock(language: string, component: ComponentType<RichBlockProps>): void {
  const key = normalize(language)
  if (!key) throw new Error('Zengin blok dili boş olamaz')
  registry.set(key, component)
}

/** Dil kayıtlıysa renderer bileşenini döndürür; değilse undefined. */
export function getRichBlock(language: string): ComponentType<RichBlockProps> | undefined {
  return registry.get(normalize(language))
}

/** Kayıtlı tüm dil anahtarları (test ve tanılama için). */
export function richBlockLanguages(): string[] {
  return [...registry.keys()]
}
