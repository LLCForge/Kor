// Saf yardımcılar: hem ana süreç hem arayüz kullanır (Node API'si yok).
import type { ChatCompletionUserMessageParam } from 'openai/resources/chat/completions'

/** read_image aracının konuşmaya eklediği görsel mesajları bu etiketle başlar. */
export const READ_IMAGE_TAG = '<read-image'

const TASK_NOTIFICATION_TAG = '<task-notification>'

export type UserContent = ChatCompletionUserMessageParam['content']

/** Kullanıcı mesajının metni (görsel parçaları atlanır); ajanın kendi eklediği görsel mesajı için null. */
export function userContentText(content: UserContent): string | null {
  const text = typeof content === 'string' ? content : content.flatMap((p) => (p.type === 'text' ? [p.text] : [])).join('\n')
  return text.startsWith(READ_IMAGE_TAG) ? null : text
}

/** Arka plan bildirimleri ve ajanın eklediği görseller kullanıcının yazdığı mesaj sayılmaz. */
export function isHumanUserText(text: string): boolean {
  return !text.startsWith(TASK_NOTIFICATION_TAG) && !text.startsWith(READ_IMAGE_TAG)
}
