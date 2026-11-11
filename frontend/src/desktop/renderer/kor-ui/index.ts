// kor-ui: Kor bileşen kütüphanesi (spec §23.5, ADR 0053). Yeni ekranlar bunlarla ve token'larla yazılır; serbest
// #hex / px değerlerini scripts/design-check.ts reddeder. Her bileşenin tüm durumları Ladle hikâyelerinde
// (`npm run ladle`, stories/*.stories.tsx).
export { Button, type ButtonProps, type ButtonVariant } from './Button.tsx'
export { Card, type CardProps } from './Card.tsx'
export { Chip, type ChipProps } from './Chip.tsx'
export { CommandPalette, ShortcutsDialog } from './CommandPalette.tsx'
export { Modal, type ModalProps } from './Modal.tsx'
export { Segmented, type SegmentOption, type SegmentedProps } from './Segmented.tsx'
export { Select, type SelectOption, type SelectProps } from './Select.tsx'
export { Toaster, toast, toasts } from './Toast.tsx'
export { Tooltip, type TooltipProps } from './Tooltip.tsx'
export { TOOLTIP_DELAY_MS, createToastStore, segmentedNext, type ToastKind } from './logic.ts'
