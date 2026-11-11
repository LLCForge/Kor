// [g35] Venice web hesabı IPC tipleri: e-posta+parola girişi, 2FA kodu, oturum durumu.
// ipc.ts'teki KorApi.g35 alanı bu türü kullanır; kanallar CHANNELS.g35*.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { VeniceWebView, VeniceWebLoginResult, G35Api } from './ipc.gen.ts'
export type { VeniceWebView, VeniceWebLoginResult, G35Api }
