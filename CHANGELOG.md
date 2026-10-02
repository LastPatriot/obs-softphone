# Changelog

All notable changes to SIP Call-In for OBS. Versions follow [semantic versioning](https://semver.org). See [docs/BRANCHING.md](docs/BRANCHING.md) for how releases are made.

## Unreleased

First public version, in preparation.

- Receive SIP calls in OBS: the caller becomes the **Call-In Caller** audio source
- Mix-minus return feed from a chosen OBS track (Track 2 by default)
- **Call-In** dock: status, caller, timer, Answer, End call, Mute caller, warnings with one-click fixes
- Settings dialog with Treasure Stream and generic SIP presets; UDP/TCP/TLS, auth username, domain, outbound proxy, SRTP, STUN
- Opus, G.722, µ-law and A-law
- Password stored in the system's secret store
- Quiet ring chime (never on air)
- Detection of another device signing in with the same account
