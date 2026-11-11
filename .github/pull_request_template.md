## Summary

<!-- What changes and why. Link the issue. The PR title is a Conventional Commit: `feat(graph): …`. -->

## Checklist

- [ ] `just test` passes; tests added or updated for new behaviour
- [ ] `just lint` is clean (clippy pedantic, TypeScript strict, design checks); `just fmt` on the files I changed
- [ ] **i18n:** every new user-facing string exists in `en`, `tr` and `fr`
- [ ] Architectural change (new crate, crate dependency, stored format, network destination) was agreed in an issue first
- [ ] No new network destinations, telemetry or secrets in logs; privacy mode still works offline
- [ ] Behaviour checked on, or reasoned for, Linux, macOS and Windows
- [ ] `backend/CHANGELOG.md` updated when user-visible
- [ ] Commits are signed off (`git commit -s`, DCO)
