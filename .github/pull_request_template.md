## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## Testing

<!-- How was it tested? e.g. cargo test, a real call on macOS/Windows/Linux, which SIP server. -->

- [ ] `cargo test --workspace` and `cargo clippy --workspace --all-targets` pass
- [ ] No passwords, phone numbers or IP addresses in code, docs or logs

<!--
For a release PR (main → release), replace the sections above with:

## Release vX.Y.Z
- [ ] `version` in Cargo.toml is X.Y.Z and CHANGELOG.md has a vX.Y.Z section
- [ ] CI is green on main
- [ ] A real call tested on macOS (and Windows/Linux if they changed)
- [ ] Merge with a merge commit, then tag the merge commit: git tag vX.Y.Z && git push origin vX.Y.Z
-->
