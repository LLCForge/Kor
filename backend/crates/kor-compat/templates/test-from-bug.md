---
name: test-from-bug
description: Turn a bug report into a failing regression test
inputs:
  - report: string # The bug report: what happened, what was expected, how to reproduce
  - file: path? # Where the bug probably lives, relative to the project root
steps:
  - Read the report and locate the code involved, starting at {{file}} when given
  - Write the smallest test that reproduces exactly the reported behaviour, in the project's test framework and style
  - Run the test and confirm it fails for the reason in the report, not for an unrelated one
  - Do not change production code; report the failing assertion and where the fix probably belongs
acceptance:
  - The new test fails on the current code for the reported reason
  - The test name describes the bug
  - No production code changed
---
Write a regression test from this bug report:

{{report}}
