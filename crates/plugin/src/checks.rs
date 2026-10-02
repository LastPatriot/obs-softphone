// SPDX-License-Identifier: GPL-2.0-or-later
//! Once-a-second checks of the OBS side, run from the return tap's audio
//! callback (DESIGN.md §3.2, §3.3, §5.1). Each pushes to the dock only when
//! its answer changes.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::{caller_source, dock, obs};

/// Set by the dock's Ui while a call is ringing or on air.
pub static IN_CALL: AtomicBool = AtomicBool::new(false);

const UNKNOWN: u8 = 2;
static MIXMINUS_BROKEN: AtomicU8 = AtomicU8::new(UNKNOWN);
static NOT_IN_SCENE: AtomicU8 = AtomicU8::new(UNKNOWN);
static MUTED: AtomicU8 = AtomicU8::new(UNKNOWN);
static MISSING: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Stores `now` and says whether it differs from the last value.
fn changed(slot: &AtomicU8, now: bool) -> bool {
    slot.swap(u8::from(now), Ordering::Relaxed) != u8::from(now)
}

/// Forget the last answers so the next tick pushes everything again
/// (after a restart of the line).
pub fn reset() {
    for slot in [&MIXMINUS_BROKEN, &NOT_IN_SCENE, &MUTED, &MISSING] {
        slot.store(UNKNOWN, Ordering::Relaxed);
    }
}

pub fn tick(mix_idx: usize) {
    let missing = caller_source::count() == 0;
    let mixminus = caller_source::on_track(mix_idx);
    // A caller nobody can hear: in a call, but no Call-In Caller is active.
    let not_in_scene = !missing && IN_CALL.load(Ordering::Relaxed) && !caller_source::any_active();
    let muted = caller_source::muted();

    let warnings_changed =
        changed(&MISSING, missing) | changed(&MIXMINUS_BROKEN, mixminus) | changed(&NOT_IN_SCENE, not_in_scene);
    if warnings_changed {
        let track = mix_idx + 1;
        let mut lines = Vec::new();
        if missing {
            lines.push("No Call-In Caller source yet: callers can't be heard in OBS.".to_string());
        }
        if not_in_scene {
            obs::log(obs::LOG_WARNING, "in a call, but no Call-In Caller source is in the current scene");
            lines.push("⚠ Caller not in the current scene, so nobody hears them.".to_string());
        }
        if mixminus {
            obs::log(obs::LOG_WARNING, &format!("Track {track} includes Call-In Caller: the caller will hear themselves"));
            lines.push(format!("⚠ Track {track} includes \"Call-In Caller\", so the caller will hear themselves."));
        }
        // One button: getting the caller heard comes first.
        let button = if missing || not_in_scene {
            dock::WarnButton::AddToScene
        } else if mixminus {
            dock::WarnButton::Fix
        } else {
            dock::WarnButton::None
        };
        dock::set_warning((!lines.is_empty()).then(|| lines.join("\n")).as_deref(), button);
    }
    if changed(&MUTED, muted) {
        dock::set_muted(muted);
    }
}
