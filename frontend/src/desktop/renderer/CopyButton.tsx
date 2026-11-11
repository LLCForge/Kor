import { Check, Copy } from 'lucide-react'
import { useState, type ReactNode } from 'react'

interface CopyButtonProps {
  getText: () => string
  label?: string
  className?: string
  /** [g3] varsayılan kopyala simgesi yerine */
  icon?: ReactNode
}

export function CopyButton({ getText, label = 'Kopyala', className = 'action-btn', icon }: CopyButtonProps) {
  const [copied, setCopied] = useState(false)

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(getText())
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }

  return (
    <button type="button" className={className} onClick={() => void copy()} aria-label={copied ? 'Kopyalandı' : label} title={label}>
      {copied ? <Check size={16} aria-hidden="true" /> : (icon ?? <Copy size={16} aria-hidden="true" />)}
    </button>
  )
}
