// Etiketler (cdesktop Tags): mesaj kutusunda @ad yazınca eklenen yeniden kullanılabilir metin parçacıkları.
// IPC sözleşmesi: YALNIZCA kanal adları + tipler (preload import eder). Handler: handlers/tags.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { Tag, TagsApi } from './ipc.gen.ts'
export type { Tag, TagsApi }
export const TAGS_CHANNELS = { list: 'tags:list', save: 'tags:save' } as const
