---
name: write-migration
description: Write a database migration with the project's migration tool
inputs:
  - change: string # What the migration must do, for example "add a nullable email column to users"
  - name: string? # Migration name; derived from the change when empty
  - reversible: bool = true # Also write the down migration
steps:
  - Find the migration tool, folder, naming and numbering this project already uses
  - Write the migration for {{change}}, named {{name}} or after the change when empty
  - When reversible is {{reversible}}, write a down migration that restores the previous schema
  - Never drop or rewrite data without a copy or backfill step; split risky changes into expand and contract migrations
  - Update models, schema files and fixtures that describe the changed tables
acceptance:
  - The migration applies cleanly with the project's migration command (run it when a local database or test container is configured)
  - The down migration restores the previous schema when reversible is true
  - The existing test suite passes
---
Write a database migration: {{change}}.
Follow the project's existing migration conventions exactly.
