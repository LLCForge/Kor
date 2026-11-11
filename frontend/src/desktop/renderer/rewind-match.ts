import type { CheckpointInfo } from '../ipc-g4.ts'
import type { ChatItem } from './chat-state.ts'

type UserItem = Extract<ChatItem, { kind: 'user' }>

/**
 * Kontrol noktalarını arayüzdeki kullanıcı mesajlarıyla metin üzerinden, sondan başa eşler:
 * her nokta bu uygulama oturumunda gönderilen bir mesaja aittir; başarısız gönderimler,
 * bildirim mesajları ve kayıttan gelen eski mesajlar eşleşmeden kalır.
 */
export function matchCheckpoints(items: readonly ChatItem[], checkpoints: readonly CheckpointInfo[]): Map<string, CheckpointInfo> {
  const map = new Map<string, CheckpointInfo>()
  const users = items.filter((i): i is UserItem => i.kind === 'user')
  let u = users.length - 1
  for (let c = checkpoints.length - 1; c >= 0 && u >= 0; c--) {
    const cp = checkpoints[c]
    if (!cp) continue
    while (u >= 0 && users[u]?.text !== cp.text) u--
    const user = users[u]
    if (!user) break
    map.set(user.id, cp)
    u--
  }
  return map
}
