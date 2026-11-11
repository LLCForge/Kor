// Diff başlığındaki 💬 N rozeti (cdesktop): o dosyaya eklenmiş inceleme yorumu sayısı.
import { MessageSquare } from 'lucide-react'
import { useReviewComments } from './review-comments.ts'

export function ReviewCount({ path }: { path: string }) {
  const n = useReviewComments().filter((c) => c.path === path).length
  if (n === 0) return null
  return (
    <span className="cdx-review-count" title={`${n} inceleme yorumu`} aria-label={`${n} inceleme yorumu`}>
      <MessageSquare size={12} aria-hidden="true" /> {n}
    </span>
  )
}
