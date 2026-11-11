// Takımlar IPC sözleşmesi: YALNIZCA kanal adları + tipler (preload import eder). Handler: handlers/teams.ts.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { TeamRoleView, TeamView, TeamsApi } from './ipc.gen.ts'
export type { TeamRoleView, TeamView, TeamsApi }
export const TEAMS_CHANNELS = { list: 'teams:list', saveUser: 'teams:saveUser' } as const
