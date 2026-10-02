# SIP Call-In for OBS

Take phone calls live on your stream. **SIP Call-In** is an OBS Studio plugin that answers calls from your phone system or VoIP provider and puts the caller into OBS as an ordinary audio source. You can adjust their volume, add filters, and meter them like any microphone. The caller hears your show through a **mix-minus** feed, so they hear you, your guests and your music, but never their own voice coming back.

It works with any standard SIP server (Asterisk, FreeSWITCH, FreePBX, 3CX, SIP trunk providers) and has a ready-made preset for **Treasure Stream** call-ins. It only **receives** calls.

**Features**
- Caller as an OBS audio source: fader, meter, filters, mute
- Mix-minus return feed from any OBS audio track (Track 2 by default)
- Auto-answer, or manual Answer with a quiet ring chime that never goes on air
- A **Call-In** dock: status, caller name, call timer, Answer, End call, Mute caller
- Warnings when something would go wrong on air (caller missing from the scene, caller hearing themselves), each with a one-click fix
- UDP, TCP or TLS; Opus, G.722, µ-law, A-law; optional SRTP, STUN and outbound proxy
- Password kept in the system's secret store (macOS Keychain, Windows Credential Manager, Linux Secret Service)

**Requirements:** OBS Studio 30 or newer. macOS 12+, Windows 10/11 (64-bit) or Ubuntu 22.04/24.04. A SIP account that can receive calls.

> **Status:** builds for macOS (universal: Apple Silicon and Intel) and Windows (x64). Linux is being prepared.

---

## Install

Download the installer for your system from the [Releases page](https://github.com/lastpatriot/obs-softphone/releases), close OBS, and run it.

The installers aren't signed with paid developer certificates, so your system asks you to confirm once:
- **macOS:** opening the installer shows "Apple could not verify…". Open **System Settings → Privacy & Security**, scroll down, click **Open Anyway**, then install as usual.
- **Windows:** "Windows protected your PC". Click **More info → Run anyway**.
- **Linux:** install the `.deb` with your software installer, or `sudo apt install ./obs-softphone-*.deb`. This version works with OBS installed from the official PPA. The Flatpak version of OBS isn't supported yet.

Start OBS. A **Call-In** dock appears (if not, open **Docks → Call-In**).

---

## Set up

1. Open **Tools → SIP Call-In…** (or **Settings…** in the dock).
2. Choose **Server type**:
   - **Treasure Stream:** copy the server, extension and password from **Admin → Call-In → Studio Softphone**. Make sure your studio's internet address is allowed under **Call-In Studio Access**, and **Studio Line** is set to *Desktop softphone* or *Automatic*.
   - **Other SIP server:** enter the server, username and password of the SIP account from your phone system or provider. Most use **UDP, port 5060**. If your provider gives you a separate auth username, a domain, an outbound proxy, a STUN server or asks for encrypted media (SRTP), open **Advanced**.
3. Click **Save**. The dock should show **● Ready** within a few seconds.
4. Add the caller to your scene: the dock shows **Add to current scene**. Or use **Sources → + → Call-In Caller**.

Use **headphones**. You hear the caller through OBS's audio monitoring, and on speakers your microphone would pick them up and send them back.

---

## Using it in a show

**Taking a call:** when a call comes in, the dock shows the caller's name and plays a quiet chime that only you hear. With **Answer calls automatically** on, it connects after a moment. Otherwise click **Answer**. During the call you get a timer, **Mute caller** and **End call**.

**What the caller hears:** the caller hears one of OBS's audio tracks, **Track 2** unless you change it in Settings. OBS puts every source on every track by default, and the plugin keeps *Call-In Caller* off that track, so out of the box the caller hears all your mics and music but not themselves.

To change what the caller hears, open the Audio Mixer's **⋮ → Advanced Audio Properties** and use the **Track 2** column:

| Source | Track 1 (your stream) | Track 2 (the caller) |
|---|---|---|
| Your mic, guest mics | ✓ | ✓ |
| Music / media | ✓ | ✓ to let the caller hear it, ☐ for voices only |
| Call-In Caller | ✓ | ☐ always |

Your stream is unaffected: OBS streams Track 1 unless you've set it up otherwise. If you already use Track 2 for something, pick a free track under **Caller hears** in Settings.

**Guests in the studio:** to let guests talk to the caller, make sure their mic sources are on the caller's track (they are by default). To let them *hear* the caller, give them headphones fed from OBS's monitoring output, for example through a headphone splitter.

**The dock's warnings:**

| Warning | What to do |
|---|---|
| No Call-In Caller source yet | Click **Add to current scene** |
| Caller not in the current scene | You switched to a scene without the caller: click **Add to current scene** |
| Track 2 includes "Call-In Caller" | The caller would hear themselves: click **Fix** |
| Another device took *account* | Something else signed in with the same SIP account, and calls go there now. Close it, then click **Take the line back** |

---

## Troubleshooting

| Dock shows | Likely cause |
|---|---|
| Wrong username or password (401/403) | Check the username and password. On Treasure Stream, a 403 can also mean your internet address isn't allowed under **Call-In Studio Access**. |
| Server refused the connection / Can't reach the server | Wrong server or port, the transport doesn't match what the server uses, or a firewall blocks it |
| Server's TLS certificate was not accepted | The server name doesn't match its certificate (wildcard certificates aren't accepted for SIP), or the certificate is invalid |
| Ready, but calls never ring | The phone system routes calls elsewhere, or another device uses the same account (watch for the "Another device" warning) |
| Caller connects but you hear nothing | The Call-In Caller source isn't in the scene on air, is muted, or OBS's monitoring device is wrong (**Settings → Audio → Advanced → Monitoring Device**) |
| Caller hears themselves | Call-In Caller is on the caller's track: click **Fix** in the dock |
| Call drops after about 30 seconds | A NAT/firewall problem between you and the server: try TCP or TLS, or set a STUN server under **Advanced** |

For details, check the OBS log (**Help → Log Files → View Current Log**) for lines starting with `[obs-softphone]`. Set `log_level` to 5 in the config file to include every SIP message (see [docs/BUILDING.md](docs/BUILDING.md#settings-file)).

---

## Building from source

See [docs/BUILDING.md](docs/BUILDING.md). The design is described in [DESIGN.md](DESIGN.md), and contributions are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Licence

GPL-2.0-or-later: see [LICENSE](LICENSE). Every source file carries an SPDX line saying so.

Third-party components in the built plugin:

| Component | Licence | How it's used |
|---|---|---|
| PJSIP (pjproject) | GPL-2.0-or-later (or commercial) | Linked statically |
| Opus | BSD-3-Clause | Linked statically |
| Qt 6, libobs, obs-frontend-api | LGPL-3.0 / GPL-2.0-or-later | Provided by OBS at runtime |
| Rust crates (serde, keyring, ...) | MIT / Apache-2.0 | Linked statically |

Anyone who receives the plugin must be able to get its source, for example from this repository.
