// Ayarlar > Kabuk için saf mantık (node testi: shell-model.test.ts): bu bilgisayarda olan kabuklar önce
// (varsayılan en başta), olmayanlar ayrı listede; `installed` yalnızca Kor bloğunun kurulu olduğu demektir.
import type { ShellStatus } from '../../ipc-shell.ts'

export type ShellRow = ShellStatus['shells'][number]

export function splitShells(shells: readonly ShellRow[]): { present: ShellRow[]; absent: ShellRow[] } {
  const present = shells.filter((s) => s.present || s.installed)
  present.sort((a, b) => Number(b.isDefault) - Number(a.isDefault))
  return { present, absent: shells.filter((s) => !s.present && !s.installed) }
}

export function shellStateText(s: ShellRow): string {
  if (s.installed) return 'Entegrasyon kurulu'
  return s.present ? 'Entegrasyon kurulu değil' : 'Bu bilgisayarda yok'
}
