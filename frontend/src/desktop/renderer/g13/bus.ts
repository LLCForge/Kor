// [g13] Geliştirici araçları penceresini her yerden açar (slash komutları, kenar çubuğu düğmesi).
// Pencere olayı kullanılır: Sidebar.tsx'te içe aktarma işareti olmadığı için düğme yalnızca olayı tetikler.
// Modül yüklenirken DOM'a dokunmaz; slash-commands.ts testlerde güvenle içe aktarılır.

export type G13Tab = 'layers' | 'env' | 'docs' | 'export'

export const G13_OPEN_EVENT = 'g13:open'

export function openG13(tab: G13Tab = 'layers'): void {
  window.dispatchEvent(new CustomEvent<G13Tab>(G13_OPEN_EVENT, { detail: tab }))
}
