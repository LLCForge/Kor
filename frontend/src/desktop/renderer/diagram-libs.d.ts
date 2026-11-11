// [g3] wavedrom için ortam tip bildirimi (paket kendi .d.ts'sini sağlamaz). Yalnızca kullandığımız DOM'suz
//      yüzey: renderAny (onml ağacı döndürür) + onml.stringify + varsayılan skin.
declare module 'wavedrom' {
  export type OnmlNode = [string, ...unknown[]]
  export function renderAny(index: number, source: Record<string, unknown>, skin: unknown, notFirstSignal?: boolean): OnmlNode
  export const waveSkin: unknown
  export const onml: { stringify(node: OnmlNode): string }
}
