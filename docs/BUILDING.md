# Building SIP Call-In from source

The plugin is a Cargo workspace (Rust) with a small C shim over PJSIP and a Qt dock in C++. See [DESIGN.md](../DESIGN.md) for how it fits together.

> Today the build scripts cover **macOS**. Windows and Linux builds are being added together with the CI workflow.

## macOS

**You need:** Rust 1.85 or newer ([rustup](https://rustup.rs)), the Xcode command-line tools (`xcode-select --install`), Homebrew with `openssl@3` and `opus` (`brew install openssl@3 opus`), and OBS Studio 30+ in `/Applications`.

```sh
git clone https://github.com/lastpatriot/obs-softphone
cd obs-softphone
scripts/bootstrap-macos.sh       # once: builds pjproject and fetches OBS's Qt headers into third_party/
cargo test --workspace           # unit tests (no OBS or network needed)
scripts/install-dev-macos.sh     # builds and installs into ~/Library/Application Support/obs-studio/plugins
```

Restart OBS after installing. `scripts/install-dev-macos.sh --release` builds an optimised version.

- `bootstrap-macos.sh` builds pjproject 2.17 (static, no video or sound devices, with Opus and OpenSSL) and downloads the obs-deps Qt 6 headers matching OBS 32.2 (hash-checked). Re-run it after changing pjproject's `config_site.h` in the script.
- The plugin links against the frameworks inside `/Applications/OBS.app`. Set `OBS_APP` to use another OBS, and `PJPROJECT_DIR`, `QT6_DEPS_DIR`, `OPENSSL_DIR`, `OPUS_DIR` to use other dependency builds.

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
