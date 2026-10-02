# SIP Call-In for OBS: Design Note

An OBS Studio plugin that **receives** SIP phone calls and puts the caller into OBS as an audio source, with a mix-minus return feed. It works with any SIP server or provider, and has a preset for Treasure Stream call-ins.

---

## 1. Goals

- **Caller audio as an OBS source** ("Call-In Caller"): mixable, filterable, metered, usable for ducking.
- **Mix-minus return feed:** the caller hears an OBS audio track the host chooses (default Track 2) that contains everything except the caller, so they hear the host, guests and music, never themselves.
- **Hands-free:** auto-answer (optional), with no clicks on the studio computer during a show.
- **A Call-In dock** in OBS: line status, the current caller, Answer / End call / Mute, warnings.
- **Works with standard SIP:** UDP, TCP or TLS; digest auth; Opus, G.722, µ-law, A-law; optional SRTP, STUN and outbound proxy.

**Non-goals:** outgoing calls, more than one caller at a time (a second call gets `486 Busy Here`), video, managing a caller queue (that belongs to the phone system).

---

## 2. How it fits in

```
Caller ──► SIP server / provider ──SIP + RTP──► SIP Call-In plugin (inside OBS)
                                                   │            ▲
                                       caller audio│            │ return feed
                                                   ▼            │
                                    "Call-In Caller" source   OBS Track N
                                                   │            ▲
                                                   └─► OBS mixer┘──► stream / recording
```

The plugin registers a SIP account, answers the incoming call, and bridges its audio to OBS in both directions. Whatever routes calls to that account (a PBX extension, a provider DID, a web call-in system) is outside the plugin.

---

## 3. Architecture

```
┌────────────────────────────── OBS process ──────────────────────────────┐
│  SIP engine (PJSIP via a thin C shim; own threads, null sound device)    │
│     account · one call · media port "obs" on the conference bridge       │
│        put_frame (caller → OBS)          get_frame (OBS → caller)        │
│              │                                     ▲                     │
│              ▼                                     │ lock-free buffer    │
│  Call-In Caller source ──► OBS mixer ──► Track N ──┘ (raw audio tap)     │
│                                                                          │
│  core: line state machine (pure Rust) ◄── events ── SIP engine, dock     │
│  Call-In dock + settings dialog (Qt) ◄── state ── core                   │
└──────────────────────────────────────────────────────────────────────────┘
```

### 3.1 SIP engine
- **PJSIP** (pjproject 2.17, `pjsua` C API) built statically with Opus and a TLS backend, without video or sound devices. A small C shim (`crates/sip/shim/sp_shim.c`) hides pjsua's large, layout-sensitive config structs from Rust.
- **No sound device:** pjsua's null device drives the conference bridge clock (20 ms frames, 48 kHz mono). It runs continuously, so OBS gets silence between calls rather than no data. A custom media port carries audio to and from OBS.
- **Media settings** (pjsua's defaults differ): echo canceller off (the return feed is clean by design), VAD off (RTP flows continuously, which keeps NAT and symmetric-RTP latching working and stops servers timing out a quiet call), 48 kHz bridge so Opus needs no resampling.
- **Codecs:** Opus, G.722, µ-law, A-law, in that order of preference. pjproject is built with `PJMEDIA_SDP_NEG_PREFER_REMOTE_CODEC_ORDER 0`, so the plugin answers with its own preference rather than the offerer's (servers often offer µ-law first).
- **NAT:** pjsua's **contact rewrite** is on (`NO_UNREG | ALWAYS_UPDATE`). pjsua learns its public address from the REGISTER response and uses it in every Contact. Some servers rewrite only the *registered* contact. With a LAN address in the Contact of the plugin's 200 OK, the server's ACK never arrives and the call drops after 32 s. Optional STUN for media.
- **Transports:** UDP, TCP, TLS (TLS 1.2/1.3, certificate verified by default against the system CA bundle). Optional outbound proxy, separate auth username and domain. SRTP (SDES) off, optional or required.
- **Calls:** answer the first call (after a 300 ms auto-answer delay, or on Answer); `486 Busy Here` for any other while one is active.
- **Threads:** every entry point registers its thread with pjlib; callbacks post events to the core and never block.

### 3.2 Call-In Caller source
- Source id `sip_callin_caller`, display name "Call-In Caller", audio only. (The legacy id `treasure_callin_caller` is registered as a hidden alias so older scenes still load.)
- `obs_source_output_audio()` straight from the media clock thread: 48 kHz mono float, 20 ms per call, timestamped with `os_gettime_ns()`. OBS buffers by timestamp and both sides use the system clock, so this direction needs no extra buffer.
- On creation: monitoring set to *Monitor and Output* (the host hears the caller; use headphones), and the source is taken off the return track. Saved scenes keep their own settings.
- Audio goes to every instance, so a duplicate behaves predictably.

### 3.3 Return feed
- `obs_add_raw_audio_callback()` taps the chosen track (`return_track`, 1–6, default 2), converted to 48 kHz mono 16-bit. OBS's tracks are 1-based in the UI and 0-based in the API.
- **Buffer** (`crates/sip/src/return_feed.rs`): lock-free, single producer (OBS audio thread, 1024-sample chunks) and single consumer (media clock, 960-sample frames), 250 ms capacity. The reader starts once 40 ms is queued and pads with silence if it runs dry (then waits for 40 ms again). If more than 120 ms is queued, it drops the oldest down to 40 ms. Delay can't build up, and drift is absorbed without resampling.
- **Mix-minus guard:** about once a second the plugin checks whether a Call-In Caller is on the return track. If so, the dock warns and offers **Fix**.

### 3.4 Levels and latency
- No gain control in the plugin: use OBS filters on Call-In Caller (Compressor, Noise Suppression, Limiter).
- Plugin-added latency is about 40 ms each way plus OBS's audio tick. OBS's own buffering comes on top.

### 3.5 Code structure
Rust, with C and C++ only where unavoidable: the pjsua shim, the Qt dock and settings dialog (OBS docks must be Qt widgets), and the ring chime (OS sound APIs).

```
crates/
  core/    softphone-core: pure Rust, no PJSIP/OBS/Qt; all state logic + tests
    line.rs      states, auto-answer, busy, "another device" detection, reconnect
    backoff.rs   retry delays (2 s → 30 s)
    caller_id.rs caller name from the INVITE
    ports.rs     SipControl, Clock, Ui: the only interfaces
    runtime.rs   the line thread and its event queue
  sip/     softphone-sip: PJSIP adapter, settings, password store, return buffer
    shim/sp_shim.c   pjsua glue, OPTIONS sniffer, bindings query, media port
  plugin/  obs-softphone (cdylib): OBS entry points, source, return tap, checks
    dock/  Qt dock, settings dialog, ring chime
  cli/     softphone-cli: the same line without OBS (developer tool)
```

**Clean architecture only where it pays:** the call-control logic sits behind three ports (`SipControl`, `Clock`, `Ui`), so timing-dependent behaviour (registration, retries, takeover detection, transport loss) is unit-tested in milliseconds with fakes. The audio path, OBS calls and the UI talk to their APIs directly: they run on real-time or UI threads and can only be tested inside OBS anyway. The core runs on one thread fed by an event queue, and nothing calls into it directly.

---

## 4. Behaviour

### 4.1 States

| State | Meaning | Dock |
|---|---|---|
| Disabled | Turned off in settings | "Off" |
| Connecting | Registering or retrying | "Connecting…" |
| Ready | Registered, waiting | "● Ready · *account*" |
| Ringing | Incoming call | Caller name (flashing), Answer if auto-answer is off |
| On air | Call connected | Caller name, timer, End call, Mute caller |
| Error | Registration failed | Plain-language reason, Retry |
| Replaced | Another device signed in with the account (§4.4) | Warning, "Take the line back" |
| Not set up | Settings incomplete | "Open settings" |

Calls are accepted whenever the line isn't disabled. A failing registration refresh doesn't stop a call from being answered.

### 4.2 Answering and ending
- Auto-answer after 300 ms (so the dock shows the caller first), or manual Answer. The caller's name comes from the INVITE's display name.
- End call sends `BYE`. A remote `BYE` returns the line to Ready.
- Disabling the line, saving new settings or closing OBS hangs up first, then unregisters, then shuts PJSIP down.

### 4.3 Reconnection
- Registration failures retry with backoff (2, 4, 8, 16, then every 30 s). Wrong password / forbidden (401, 403, 407) waits for Retry instead.
- If a TCP/TLS connection drops during a call, the plugin ends the call: in-dialog requests would go to the old connection, so the call can't survive a reconnect. It then re-registers.

### 4.4 "Another device signed in with this account"
Many servers allow one device per account and silently replace the older registration. They don't tell the replaced device. The plugin infers it:
1. **Trigger:** servers that keep registrations alive send `OPTIONS` to the registered contact (typically every 30 s). A small pjsip module sees them before pjsua answers. If none arrive for 75 s, the plugin checks.
2. **Confirm:** a `REGISTER` with no Contact header asks the server for its current bindings without changing them (RFC 3261 §10.2.4). If another contact holds the account: **Replaced**. If ours is still there, or the answer is unclear: stay Ready (a network blip). If none: register again.

In Replaced the plugin stops all registration refreshes, so it never silently takes the line back. "Take the line back" re-registers once (rate-limited to avoid ping-pong). Servers that never send OPTIONS just get one bindings query every 75 s. The feature can be turned off in Settings.

---

## 5. User interface

### 5.1 Call-In dock
Status, caller and timer; Answer, End call, Retry, Take the line back, Mute caller (mirrors the mixer's mute); Auto-answer (saved); a warning row with one action button:
- **No Call-In Caller source** or **caller not in the current scene** (during a call): *Add to current scene*.
- **Return track includes Call-In Caller:** *Fix* (unticks that track for the source).

**Ring chime** (on by default): a soft two-tone chime when a call rings, repeated every 2 s while waiting for a manual Answer. It's played through the OS sound API (AudioServices on macOS, PlaySound on Windows), so it never reaches OBS's mix, the stream or the caller.

### 5.2 Settings dialog
**Tools → SIP Call-In…**, the dock's *Settings…*, or *Open settings*.
- **Server type** preset: *Treasure Stream* (TLS, 5061, extension 101) or *Other SIP server* (UDP, 5060). The preset only fills in defaults and hints, never credentials.
- Connect, server, transport, port, username, password, *Caller hears* (track), auto-answer, ring chime, takeover warning.
- **Advanced:** auth username, domain, outbound proxy, SRTP, STUN server, TLS verification.

**Save** applies immediately by restarting the line (PJSIP included) without restarting OBS.

---

## 6. Security
- **Password:** in the OS secret store (macOS Keychain, Windows Credential Manager, Linux Secret Service) under service `obs-softphone`, account `<username>@<server>`. `config.json` (OBS's per-plugin config folder, mode 0600) holds everything else. A password found in the file is moved to the store. If no store is available, it stays in the file and the log says why. Settings never go into the scene collection, because people export and share scene collections.
- **TLS:** certificates verified by default. PJSIP rejects **wildcard** certificates for SIP (RFC 5922 §7.2), so servers need a certificate for their exact host name.
- **Media:** plain RTP unless SRTP is enabled.
- The plugin never places calls.

---

## 7. Notes for server operators
- **One device per account** makes the takeover warning meaningful. With several devices allowed, the plugin just stays Ready.
- **Keep-alive OPTIONS** (Asterisk `qualify_frequency`) let the plugin notice takeovers quickly.
- **Symmetric RTP** (Asterisk `rtp_symmetric`, `force_rport`, `rewrite_contact`) handles studios behind NAT. The plugin sends RTP continuously, so an **RTP timeout** (Asterisk `rtp_timeout`) is safe and cleans up a call whose studio vanished.

---

## 8. Build and distribution
- **Cargo workspace.** `build.rs` scripts compile the C shim, the Qt dock and the chime with the `cc` crate and link pjproject; the OBS entry points are exported from Rust.
- **macOS dev setup:** `scripts/bootstrap-macos.sh` builds pjproject into `third_party/` and fetches the obs-deps Qt 6 headers that match OBS. `scripts/install-dev-macos.sh` builds and installs `obs-softphone.plugin`.
- **TLS backend:** dev builds link Homebrew OpenSSL statically (with the system CA bundle passed explicitly). Release builds should use the OS's own TLS through PJSIP's backends (Apple on macOS, Schannel on Windows, OpenSSL/GnuTLS from the distribution on Linux).
- **Targets:** OBS 30+ on macOS (universal), Windows x64, Ubuntu 22.04/24.04. Built by CI per platform; installers `.pkg`, `.exe`, `.deb`.

---

## 9. Testing
- **Unit tests** (`core`, `sip`): every state change, auto-answer, busy, backoff, takeover detection and take-back limits, transport loss, settings migration and password store fallback, the return buffer (ordering, priming, padding, wrap-around, overflow, two threads).
- **Examples** (no OBS): `media_clock` (20 ms cadence, in-process restart), `transport_check` (UDP/TCP/TLS registration against a server), `bindings_query` (read-only takeover check), `keychain_check`.
- **Manual, per platform:** registration (good/bad password, unreachable server, bad certificate); calls (auto/manual answer, busy, caller hangs up, End call, network drop); audio (Opus/G.722/µ-law/A-law, 2-hour drift, mix-minus, warnings, monitoring); takeover and take-back; Settings → Save while idle.

---

## 10. Status and decisions
- **Done:** registration and calls, caller source, return feed, dock, settings dialog, password store, ring chime, takeover detection, generic SIP options.
- **Next:** CI builds for macOS, Windows and Linux; installers; install guide.
- **Licence:** GPL-2.0-or-later (see README for third-party components).
- **Default return track:** Track 2. OBS puts every source on every track by default and the plugin keeps Call-In Caller off Track 2, so mix-minus works with no setup.
- **Receive-only** by design.
