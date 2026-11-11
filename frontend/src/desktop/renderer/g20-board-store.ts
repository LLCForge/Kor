// [g20] Beyaz tahta penceresinin aç/kapa deposu ve yerel kayıtlar (React yok: slash-commands.ts de kullanır).
//      Tahta ve anlık görüntüler yalnızca bu cihazda, try/catch'li localStorage'da tutulur (#119 deseni):
//      paylaşılmaz, depolama kapalı/dolu ise yalnızca bu oturumda yaşar ve kullanıcıya bildirilir.
import { parseBoard, type Board } from './g20-board.ts'

let open = false
const listeners = new Set<() => void>()

export function openBoard(): void {
  open = true
  for (const l of listeners) l()
}

export function closeBoard(): void {
  open = false
  for (const l of listeners) l()
}

export function subscribeBoard(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export const isBoardOpen = () => open

const CURRENT_KEY = 'g20:board:current'
const SNAPSHOTS_KEY = 'g20:board:snapshots'
export const MAX_SNAPSHOTS = 20

export interface Snapshot {
  id: string
  at: number
  board: Board
}

function read(key: string): unknown {
  try {
    const raw = window.localStorage.getItem(key)
    return raw === null ? null : (JSON.parse(raw) as unknown)
  } catch (err) {
    console.warn('Tuval kaydı okunamadı:', key, err)
    return null
  }
}

/** @returns kaydedilebildiyse true */
function write(key: string, value: unknown): boolean {
  try {
    window.localStorage.setItem(key, JSON.stringify(value))
    return true
  } catch (err) {
    console.warn('Tuval kaydedilemedi:', key, err)
    return false
  }
}

export const loadCurrentBoard = (): Board => parseBoard(read(CURRENT_KEY))
export const saveCurrentBoard = (board: Board): boolean => write(CURRENT_KEY, board)

export function loadSnapshots(): Snapshot[] {
  const raw = read(SNAPSHOTS_KEY)
  if (!Array.isArray(raw)) return []
  return raw.slice(0, MAX_SNAPSHOTS).flatMap((v: unknown): Snapshot[] => {
    const s = v as { id?: unknown; at?: unknown; board?: unknown } | null
    return s && typeof s.id === 'string' && typeof s.at === 'number' ? [{ id: s.id, at: s.at, board: parseBoard(s.board) }] : []
  })
}

export const saveSnapshots = (snapshots: Snapshot[]): boolean => write(SNAPSHOTS_KEY, snapshots.slice(0, MAX_SNAPSHOTS))
