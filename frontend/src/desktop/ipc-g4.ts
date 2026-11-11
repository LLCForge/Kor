// Grup 4 (dosya, diff, editör) IPC sözleşmesi. ipc.ts'teki KorApi.g4 alanı bu türü kullanır.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { G4FileEntry, FileListing, FilePreview, SaveResult, RewindAction, RewindPreview, RewindChange, RewindResult, ExternalStatus, G4Event, OpenedWith, G4Api } from './ipc.gen.ts'
export type { G4FileEntry, FileListing, FilePreview, SaveResult, RewindAction, RewindPreview, RewindChange, RewindResult, ExternalStatus, G4Event, OpenedWith, G4Api }
import type { CheckpointInfo } from '../core/checkpoints.ts'
import type { Diagnostic, DiagnosticsReport, ToolReport } from '../core/diagnostics.ts'
import type { EditorId } from '../core/external-editor.ts'

export type { CheckpointInfo, Diagnostic, DiagnosticsReport, EditorId, ToolReport }

export const G4_CHANNELS = {
  listDir: 'g4:fs:listDir',
  listAllFiles: 'g4:fs:listAll',
  preview: 'g4:fs:preview',
  writeFile: 'g4:fs:write',
  createFile: 'g4:fs:createFile',
  createDir: 'g4:fs:createDir',
  rename: 'g4:fs:rename',
  trash: 'g4:fs:trash',
  watch: 'g4:fs:watch',
  detectEditors: 'g4:editor:detect',
  openInEditor: 'g4:editor:open',
  listCheckpoints: 'g4:checkpoint:list',
  previewRewind: 'g4:checkpoint:preview',
  rewind: 'g4:checkpoint:rewind',
  runDiagnostics: 'g4:diagnostics:run',
  event: 'g4:event',
} as const
