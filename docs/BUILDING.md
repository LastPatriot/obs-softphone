# Building SIP Call-In from source

The plugin is a Cargo workspace (Rust) with a small C shim over PJSIP and a Qt dock in C++. See [DESIGN.md](../DESIGN.md) for how it fits together.

> Today the build scripts cover **macOS**. Windows and Linux builds are being added together with the CI workflow.

## macOS

**You need:** Rust 1.85 or newer ([rustup](https://rustup.rs)) and the Xcode command-line tools (`xcode-select --install`). OBS Studio 30+ in `/Applications` is used if present. Nothing from Homebrew is needed.

```sh
git clone https://github.com/lastpatriot/obs-softphone
cd obs-softphone
scripts/bootstrap-macos.sh       # once: builds Opus and pjproject and fetches OBS's Qt headers into third_party/
cargo test --workspace           # unit tests (no OBS or network needed)
scripts/install-dev-macos.sh     # builds and installs into ~/Library/Application Support/obs-studio/plugins
```

Restart OBS after installing. `scripts/install-dev-macos.sh --release` builds an optimised version.

**Universal build and installer** (what CI does):

```sh
ARCHS="arm64 x86_64" scripts/bootstrap-macos.sh
scripts/fetch-obs-macos.sh arm64 x86_64        # OBS's arm64 and Intel frameworks (libobs is single-arch)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
ARCHS="arm64 x86_64" scripts/package-macos.sh  # dist/: plugin bundle, .zip and .pkg
```

`package-macos.sh` builds each architecture, joins them with `lipo`, ad-hoc signs the bundle, and builds an unsigned installer that puts it in `~/Library/Application Support/obs-studio/plugins`. `install-dev-macos.sh` uses it for a host-only debug build.

**CI:** `.github/workflows/build.yml` runs these steps on GitHub's Macs for every push and pull request: clippy with warnings as errors, tests, the media-clock check, and the universal package (checked for both architectures, no OpenSSL, the right install path). The `.pkg` and `.zip` are attached to each run. Pushing a `v*` tag also creates a **draft** GitHub Release with them and a `SHA256SUMS.txt`.

- `bootstrap-macos.sh` builds Opus 1.5.2 and pjproject 2.17 (static, no video or sound devices) into `third_party/opus-<arch>` and `third_party/pjproject-<arch>`, and downloads the obs-deps Qt 6 headers matching OBS 32.2 (all hash-checked). TLS uses Apple's **Network.framework** (TLS 1.3, the system trust store), so there's no OpenSSL. It builds the host's architecture; `ARCHS="arm64 x86_64"` builds both. Bump `BUILD_REV` in the script after changing pjproject's options.
- The plugin links against OBS's own frameworks: `third_party/obs-app-<arch>` if present (from `fetch-obs-macos.sh`), else `/Applications/OBS.app`. Override with `OBS_APP_ARM64` / `OBS_APP_X86_64` or `OBS_APP`, and `PJPROJECT_DIR`, `OPUS_DIR`, `QT6_DEPS_DIR` for other dependency builds.

## Developer tools

Everything below runs without OBS.

| Command | What it does |
|---|---|
| `cargo run -p softphone-cli -- <config.json> [--tone]` | Runs the line in a terminal: registers, answers calls, prints the state and the caller's level once a second. `--tone` plays the caller a 440 Hz tone. Commands: `a` answer, `e` end, `r` retry, `t` take the line back, `on`/`off`, `auto on`/`auto off`, `q` quit. |
| `cargo run -p softphone-sip --example transport_check -- <server> <udp\|tcp\|tls> [port]` | Registers a throwaway account to prove a transport works (a 401/403 is a pass). |
| `cargo run -p softphone-sip --example bindings_query -- <config.json>` | Read-only: asks the server who currently holds the account. |
| `cargo run -p softphone-sip --example media_clock` | Checks the 20 ms audio clock, including a restart. |
| `cargo run -p softphone-sip --example keychain_check` | Round-trips a throwaway secret through the OS secret store. |

The CLI and `bindings_query` register with, or query, the real server. If the server allows one device per account, they take the account away from OBS while they run.

## Settings file

The settings dialog writes `config.json` in OBS's plugin config folder:
- macOS: `~/Library/Application Support/obs-studio/plugin_config/obs-softphone/config.json`
- Windows: `%APPDATA%\obs-studio\plugin_config\obs-softphone\config.json`
- Linux: `~/.config/obs-studio/plugin_config/obs-softphone/config.json`

The password isn't in it: it's kept in the system's secret store (service `obs-softphone`, account `<username>@<server>`). A password found in the file is moved there on load. If no secret store is available, it stays in the file.

```json
{
  "preset": "generic",
  "enabled": true,
  "server": "sip.example.com",
  "transport": "udp",
  "port": 5060,
  "username": "1001",
  "auth_username": "",
  "domain": "",
  "outbound_proxy": "",
  "srtp": "off",
  "stun_server": "",
  "verify_tls": true,
  "ca_file": null,
  "auto_answer": true,
  "auto_answer_delay_ms": 300,
  "ring_chime": true,
  "replaced_detection": true,
  "return_track": 2,
  "log_level": 3
}
```

| Field | Meaning |
|---|---|
| `preset` | `generic` or `treasure_stream`: only changes defaults and hints in the dialog |
| `transport` | `udp`, `tcp` or `tls` |
| `auth_username`, `domain` | Empty means "same as `username`" and "same as `server`" |
| `outbound_proxy`, `stun_server` | `host` or `host:port`, empty for none |
| `srtp` | `off`, `optional` or `required` |
| `ca_file` | CA bundle for TLS. `null` uses the system bundle. |
| `replaced_detection` | Warn when another device signs in with the same account |
| `return_track` | 1–6: the OBS track the caller hears |
| `log_level` | pjlib 0–6. 4+ writes SIP details to the OBS log, 5 includes full SIP messages. |

## Code layout

| Crate | What |
|---|---|
| `crates/core` | Call-control logic in plain Rust behind three interfaces (`SipControl`, `Clock`, `Ui`); most unit tests live here |
| `crates/sip` | PJSIP adapter (C shim + Rust), settings, password store, return-feed buffer |
| `crates/plugin` | The OBS plugin: entry points, Call-In Caller source, return tap, checks, Qt dock, settings dialog, chime |
| `crates/cli` | The developer CLI |
