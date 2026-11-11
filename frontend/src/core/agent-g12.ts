// [g12] Ajan döngüsü kancası: toplu silme onay kanalını oturumun kabuk sahibine bağlar (bkz. bulk-delete-g12.ts).
import { randomUUID } from 'node:crypto'
import type { AgentIO, Session } from './agent.ts'
import { describeBulkDelete, setBulkDeleteConfirmer } from './bulk-delete-g12.ts'

export function g12BeforeModelCall(session: Session, io: AgentIO): void {
  // Masaüstü dosya listeli ayrı bir diyalog sunar; CLI'da genel onay sorusu uyarı metniyle kullanılır.
  const viaPrompt = async (request: Parameters<typeof describeBulkDelete>[0]) =>
    (await io.confirm(randomUUID(), 'bash', request.command, describeBulkDelete(request))) !== 'no'
  setBulkDeleteConfirmer(session.shells, io.g12ConfirmBulkDelete ?? viaPrompt)
}
