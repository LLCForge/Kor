// Yazmadan önce dosya bazında onay (§12.3, ADR 0030). Rust: kor_agent::review + src-tauri chat.rs.
// `confirm` olayı `files` ve `destructive` taşır; yanıt `approve(id, approval, files?)` ile döner.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ReviewFile, FileDecision, FileDecisions, ConfirmReviewFields } from './ipc.gen.ts'
export type { ReviewFile, FileDecision, FileDecisions, ConfirmReviewFields }

/** Çekirdeğin yıkıcılık neden anahtarları (kor-agent permissions.rs / kor-tools review_risk). */
export const DESTRUCTIVE_REASONS: Readonly<Record<string, string>> = {
  'rm-rf': 'klasörü içindekilerle siler',
  'force-push': 'uzak dalın geçmişini zorla yazar',
  'reset-hard': 'kaydedilmemiş değişiklikleri atar',
  'git-clean': 'izlenmeyen dosyaları siler',
  'pipe-to-shell': 'indirilen betiği doğrudan çalıştırır',
  disk: 'diski biçimlendirir veya üzerine yazar',
  'fork-bomb': 'sistemi kilitler',
  shutdown: 'bilgisayarı kapatır veya yeniden başlatır',
  chmod: 'sistem dosyalarının izinlerini değiştirir',
  sudo: 'yönetici yetkisiyle çalışır',
  'sql-drop': 'veritabanı tablosunu veya verisini siler',
  'mcp-write': 'veri değiştiren bir MCP aracı (sunucusu yıkıcı olarak işaretlemiş)',
  migration: 'veritabanı şemasını değiştirir (migration)',
  'delete-file': 'dosya siler',
  'overwrite-unread': 'bu oturumda okunmamış bir dosyanın üzerine yazar',
}

export const destructiveReason = (key: string): string => DESTRUCTIVE_REASONS[key] ?? key
