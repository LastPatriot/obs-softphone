# Contributing

Thanks for helping. Bug reports, fixes and improvements are welcome.

**Reporting a problem:** open an issue with your OS and OBS version, the server type (Treasure Stream, Asterisk, FreePBX, a provider...), what you expected and what happened. Attach the `[obs-softphone]` lines from the OBS log (**Help → Log Files**), ideally with `log_level` 5 (see [docs/BUILDING.md](docs/BUILDING.md#settings-file)). **Remove passwords, phone numbers and IP addresses first.**

**Sending a change:** branch from `main` (`feature/<topic>` or `fix/<topic>`) and open a pull request into `main`. `release` only changes through release and hotfix pull requests: see [docs/BRANCHING.md](docs/BRANCHING.md). Add a line to *Unreleased* in [CHANGELOG.md](CHANGELOG.md) for anything users would notice.
- Build and test as described in [docs/BUILDING.md](docs/BUILDING.md). `cargo test --workspace` and `cargo clippy --workspace --all-targets` should pass with no warnings.
- Keep call-control logic in `crates/core` and cover it with unit tests (the fakes in `crates/core/src/line/tests.rs` make timing tests fast).
- Code on audio threads (the media clock and OBS's audio callback) must not block or allocate per frame.
- Match the surrounding style. Keep the SPDX line at the top of each source file.
- The plugin is receive-only by design. Please open an issue before working on larger features.

By contributing, you agree that your contribution is licensed under GPL-2.0-or-later, the same as the project.
