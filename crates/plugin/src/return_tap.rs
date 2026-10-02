// SPDX-License-Identifier: GPL-2.0-or-later
//! The caller's return feed (DESIGN.md §3.3): taps one OBS audio track
//! (everything except the caller), converted to 48 kHz mono 16-bit, into the
//! [`ReturnFeed`] that PJSIP reads from.

use std::ffi::{c_int, c_void};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use softphone_sip::{FeedStats, ReturnFeed};

use crate::{caller_source, checks, obs};

/// `struct audio_convert_info`.
#[repr(C)]
struct ConvertInfo {
    samples_per_sec: u32,
    format: c_int,
    speakers: c_int,
    allow_clipping: bool,
}

/// `struct audio_data`.
#[repr(C)]
struct AudioData {
    data: [*const u8; 8],
    frames: u32,
    timestamp: u64,
}

const AUDIO_FORMAT_16BIT: c_int = 2;
const SPEAKERS_MONO: c_int = 1;

type Callback = extern "C" fn(*mut c_void, usize, *mut AudioData);

unsafe extern "C" {
    fn obs_add_raw_audio_callback(mix_idx: usize, conversion: *const ConvertInfo, cb: Callback, param: *mut c_void);
    fn obs_remove_raw_audio_callback(mix_idx: usize, cb: Callback, param: *mut c_void);
}

static FEED: LazyLock<ReturnFeed> = LazyLock::new(ReturnFeed::default);
const NOT_TAPPED: usize = usize::MAX;
static MIX: AtomicUsize = AtomicUsize::new(NOT_TAPPED);

/// UI thread. Starts tapping `mix_idx` (0-based: Track 2 is 1).
pub fn start(mix_idx: usize) {
    caller_source::set_return_mix(mix_idx);
    let conv = ConvertInfo {
        samples_per_sec: softphone_sip::SAMPLE_RATE,
        format: AUDIO_FORMAT_16BIT,
        speakers: SPEAKERS_MONO,
        allow_clipping: false,
    };
    // SAFETY: libobs copies `conv`; the callback is a plain function.
    unsafe { obs_add_raw_audio_callback(mix_idx, &conv, on_audio, std::ptr::null_mut()) };
    MIX.store(mix_idx, Ordering::SeqCst);
    obs::log(obs::LOG_INFO, &format!("return feed: caller hears Track {}", mix_idx + 1));
}

/// UI thread, at unload. After this returns no more audio is written.
pub fn stop() {
    let mix = MIX.swap(NOT_TAPPED, Ordering::SeqCst);
    if mix != NOT_TAPPED {
        unsafe { obs_remove_raw_audio_callback(mix, on_audio, std::ptr::null_mut()) };
    }
}

/// PJSIP media clock: what the caller hears next.
pub fn fill(out: &mut [i16]) {
    FEED.pop(out);
}

static TICKS: AtomicU32 = AtomicU32::new(0);

/// OBS audio thread, about every 21 ms (1024 samples).
extern "C" fn on_audio(_param: *mut c_void, mix_idx: usize, data: *mut AudioData) {
    // SAFETY: libobs passes valid audio_data for the duration of the call;
    // with our conversion, plane 0 holds `frames` mono i16 samples.
    let samples = unsafe {
        let d = &*data;
        if d.data[0].is_null() {
            return;
        }
        std::slice::from_raw_parts(d.data[0].cast::<i16>(), d.frames as usize)
    };
    FEED.push(samples);

    let tick = TICKS.fetch_add(1, Ordering::Relaxed);
    if tick % 47 == 0 {
        checks::tick(mix_idx);
    }
    if tick % 470 == 0 {
        log_stats();
    }
}

/// About every 10 s: log the feed's health when it changed.
fn log_stats() {
    static LAST: std::sync::Mutex<FeedStats> = std::sync::Mutex::new(FeedStats { queued: 0, underruns: 0, skipped: 0 });
    let s = FEED.stats();
    let mut last = LAST.lock().unwrap();
    if (s.underruns, s.skipped) != (last.underruns, last.skipped) {
        obs::log(
            obs::LOG_INFO,
            &format!(
                "return feed: {} ms queued, {} underruns, {} ms dropped (total)",
                s.queued / 48,
                s.underruns,
                s.skipped / 48
            ),
        );
    }
    *last = s;
}
