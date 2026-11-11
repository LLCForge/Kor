// ipc.ts `system`, `computer`, `layout` ad alanlarını tiplemiyor; tipler ipc-system.ts içinde (ADR 0059).
import type { ComputerApi, LayoutApi, SystemApi } from '../../../ipc-system.ts'
import { api } from '../../api.ts'

const typed = api as unknown as { system: SystemApi; computer: ComputerApi; layout: LayoutApi }
export const systemApi = typed.system
export const computerApi = typed.computer
export const layoutApi = typed.layout

/** Sağ alttaki karttan "Temizle": Sistem görünümü açılır ve temizlik önerilerini yükler. */
export const CLEAN_EVENT = 'ksys:clean'
