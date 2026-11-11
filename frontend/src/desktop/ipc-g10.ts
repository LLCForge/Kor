// Grup 10 (güvenilirlik ve hata kurtarma) IPC sözleşmesi. ipc.ts'teki KorApi.g10 alanı bu türü kullanır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { RecoverableTurn, RestoreResult, G10Api } from './ipc.gen.ts'
export type { RecoverableTurn, RestoreResult, G10Api }
import type { G10Event } from '../core/resilience-g10.ts'
import type { CheckItem, CheckStatus } from '../core/self-check-g10.ts'
import type { DeadLetter } from '../core/turn-journal-g10.ts'

export type { CheckItem, CheckStatus, DeadLetter, G10Event }

// Kanal adları ipc.ts CHANNELS içinde ([g10] kanallar): preload yalnızca onu içe aktarır.
