// SPDX-License-Identifier: GPL-2.0-or-later
//! The "Call-In Caller" audio source (DESIGN.md §3.2).
//!
//! PJSIP's media clock delivers 20 ms of caller audio (silence between calls)
//! and [`push`] hands it to every instance with `obs_source_output_audio`.
//! OBS buffers and mixes it by timestamp, so no ring buffer is needed in this
//! direction.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::obs;

/// The first fields of libobs's `struct obs_source_info` (OBS 30–32).
/// `obs_register_source_s` copies `size` bytes and zeroes the rest.
#[repr(C)]
struct SourceInfo {
    id: *const c_char,
    kind: c_int,
    output_flags: u32,
    get_name: extern "C" fn(*mut c_void) -> *const c_char,
    create: extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    destroy: extern "C" fn(*mut c_void),
}

// SAFETY: only static strings and function pointers.
unsafe impl Sync for SourceInfo {}

/// `struct obs_source_audio`.
#[repr(C)]
struct SourceAudio {
    data: [*const u8; 8], // MAX_AV_PLANES
    frames: u32,
    speakers: c_int,
    format: c_int,
    samples_per_sec: u32,
    timestamp: u64,
}

const OBS_SOURCE_TYPE_INPUT: c_int = 0;
const OBS_SOURCE_AUDIO: u32 = 1 << 1;
const OBS_SOURCE_DO_NOT_DUPLICATE: u32 = 1 << 7;
const OBS_SOURCE_DEPRECATED: u32 = 1 << 8;
const SPEAKERS_MONO: c_int = 1;
const AUDIO_FORMAT_FLOAT: c_int = 4;
const OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT: c_int = 2;

unsafe extern "C" {
    fn obs_register_source_s(info: *const SourceInfo, size: usize);
    fn obs_source_output_audio(source: *mut c_void, audio: *const SourceAudio);
    fn obs_source_set_monitoring_type(source: *mut c_void, kind: c_int);
    fn obs_source_get_audio_mixers(source: *const c_void) -> u32;
    fn obs_source_set_audio_mixers(source: *mut c_void, mixers: u32);
    fn obs_source_active(source: *const c_void) -> bool;
    fn obs_source_muted(source: *const c_void) -> bool;
    fn obs_source_set_muted(source: *mut c_void, muted: bool);
    fn obs_source_create(id: *const c_char, name: *const c_char, settings: *mut c_void, hotkeys: *mut c_void) -> *mut c_void;
    fn obs_source_get_ref(source: *mut c_void) -> *mut c_void;
    fn obs_source_release(source: *mut c_void);
    fn obs_get_source_by_name(name: *const c_char) -> *mut c_void;
    fn obs_scene_from_source(source: *const c_void) -> *mut c_void;
    fn obs_scene_add(scene: *mut c_void, source: *mut c_void) -> *mut c_void;
    fn obs_frontend_get_current_scene() -> *mut c_void;
    fn os_gettime_ns() -> u64;
}

static INFO: SourceInfo = SourceInfo {
    id: c"sip_callin_caller".as_ptr(),
    kind: OBS_SOURCE_TYPE_INPUT,
    output_flags: OBS_SOURCE_AUDIO | OBS_SOURCE_DO_NOT_DUPLICATE,
    get_name,
    create,
    destroy,
};

struct SourcePtr(*mut c_void);
// SAFETY: obs_source_t is used from any thread by design; entries are only
// used while SOURCES is locked, and destroy removes them under the lock.
unsafe impl Send for SourcePtr {}

/// Live instances. Usually one; every instance gets the audio.
static SOURCES: Mutex<Vec<SourcePtr>> = Mutex::new(Vec::new());

/// The track the caller hears (0-based), set by the return tap.
static RETURN_MIX: AtomicUsize = AtomicUsize::new(1);

pub fn set_return_mix(mix_idx: usize) {
    RETURN_MIX.store(mix_idx, Ordering::Relaxed);
}

pub fn return_mix() -> usize {
    RETURN_MIX.load(Ordering::Relaxed)
}

/// True if any Call-In Caller is on `mix_idx`, i.e. the caller would hear
/// themselves on the return feed.
pub fn on_track(mix_idx: usize) -> bool {
    let sources = SOURCES.lock().unwrap();
    // SAFETY: sources in SOURCES are alive (locked).
    sources.iter().any(|s| unsafe { obs_source_get_audio_mixers(s.0) } & (1 << mix_idx) != 0)
}

/// How many Call-In Caller sources exist.
pub fn count() -> usize {
    SOURCES.lock().unwrap().len()
}

/// UI thread (the dock's "Add to current scene"): puts the existing
/// Call-In Caller into the current scene, or creates one if there is none.
pub fn add_to_current_scene() {
    // SAFETY: libobs/frontend calls on the UI thread; every reference taken
    // here is released here.
    unsafe {
        let scene_source = obs_frontend_get_current_scene();
        if scene_source.is_null() {
            return;
        }
        let scene = obs_scene_from_source(scene_source);
        // Take a reference under the lock, but call into OBS without it:
        // creating a source calls our `create`, which locks SOURCES.
        let existing = SOURCES.lock().unwrap().first().map(|s| obs_source_get_ref(s.0));
        let source = match existing {
            Some(s) if !s.is_null() => s,
            _ => obs_source_create(INFO.id, free_name().as_ptr(), std::ptr::null_mut(), std::ptr::null_mut()),
        };
        if !scene.is_null() && !source.is_null() {
            obs_scene_add(scene, source);
            obs::log(obs::LOG_INFO, "added Call-In Caller to the current scene");
        }
        if !source.is_null() {
            obs_source_release(source);
        }
        obs_source_release(scene_source);
    }
}

/// "Call-In Caller", or "Call-In Caller 2", ... if that name is taken.
fn free_name() -> std::ffi::CString {
    (1..)
        .map(|n| if n == 1 { "Call-In Caller".to_string() } else { format!("Call-In Caller {n}") })
        .map(|name| std::ffi::CString::new(name).unwrap())
        .find(|name| {
            // SAFETY: returns a new reference or NULL.
            let found = unsafe { obs_get_source_by_name(name.as_ptr()) };
            if !found.is_null() {
                unsafe { obs_source_release(found) };
            }
            found.is_null()
        })
        .expect("a free name")
}

/// True if some Call-In Caller is in the program output (current scene).
pub fn any_active() -> bool {
    let sources = SOURCES.lock().unwrap();
    // SAFETY: sources in SOURCES are alive (locked).
    sources.iter().any(|s| unsafe { obs_source_active(s.0) })
}

/// True if every Call-In Caller is muted (and there is at least one).
pub fn muted() -> bool {
    let sources = SOURCES.lock().unwrap();
    // SAFETY: as above.
    !sources.is_empty() && sources.iter().all(|s| unsafe { obs_source_muted(s.0) })
}

/// UI thread: the dock's Mute caller button, matching the mixer's mute.
pub fn toggle_mute() {
    let mute = !muted();
    let sources = SOURCES.lock().unwrap();
    for s in sources.iter() {
        // SAFETY: as above.
        unsafe { obs_source_set_muted(s.0, mute) };
    }
}

/// UI thread: take every Call-In Caller off `mix_idx` (the dock's "Fix").
pub fn remove_from_track(mix_idx: usize) {
    let sources = SOURCES.lock().unwrap();
    for s in sources.iter() {
        // SAFETY: as above.
        unsafe { obs_source_set_audio_mixers(s.0, obs_source_get_audio_mixers(s.0) & !(1 << mix_idx)) };
    }
    obs::log(obs::LOG_INFO, &format!("took Call-In Caller off Track {}", mix_idx + 1));
}

/// The id used before the plugin became generic: still loads saved
/// scenes, but is hidden from the Add Source menu.
static LEGACY_INFO: SourceInfo = SourceInfo {
    id: c"treasure_callin_caller".as_ptr(),
    kind: OBS_SOURCE_TYPE_INPUT,
    output_flags: OBS_SOURCE_AUDIO | OBS_SOURCE_DO_NOT_DUPLICATE | OBS_SOURCE_DEPRECATED,
    get_name,
    create,
    destroy,
};

pub fn register() {
    // SAFETY: both are 'static and laid out like the struct's prefix.
    unsafe {
        obs_register_source_s(&INFO, size_of::<SourceInfo>());
        obs_register_source_s(&LEGACY_INFO, size_of::<SourceInfo>());
    }
}

extern "C" fn get_name(_type_data: *mut c_void) -> *const c_char {
    c"Call-In Caller".as_ptr()
}

extern "C" fn create(_settings: *mut c_void, source: *mut c_void) -> *mut c_void {
    // Let the host hear the caller (§3.2). When a saved scene loads, OBS
    // applies the saved monitoring type after create, so a host's own
    // choice is kept.
    // Keep a new source off the return track, so the caller never hears
    // themselves (mix-minus, §3.3). Saved mixers also win on load.
    unsafe {
        obs_source_set_monitoring_type(source, OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT);
        let mix = RETURN_MIX.load(Ordering::Relaxed);
        obs_source_set_audio_mixers(source, obs_source_get_audio_mixers(source) & !(1 << mix));
    }
    SOURCES.lock().unwrap().push(SourcePtr(source));
    obs::log(obs::LOG_INFO, "Call-In Caller source created");
    // OBS treats NULL as failure; the source pointer doubles as our data.
    source
}

extern "C" fn destroy(data: *mut c_void) {
    SOURCES.lock().unwrap().retain(|s| s.0 != data);
}

thread_local! {
    static FLOAT_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// PJSIP media clock thread, every 20 ms: 48 kHz mono samples.
pub fn push(samples: &[i16]) {
    let sources = SOURCES.lock().unwrap();
    if sources.is_empty() {
        return;
    }
    FLOAT_BUF.with_borrow_mut(|buf| {
        to_float(samples, buf);
        let mut data = [std::ptr::null(); 8];
        data[0] = buf.as_ptr().cast();
        let audio = SourceAudio {
            data,
            frames: buf.len() as u32,
            speakers: SPEAKERS_MONO,
            format: AUDIO_FORMAT_FLOAT,
            samples_per_sec: softphone_sip::SAMPLE_RATE,
            // SAFETY: plain libobs clock read.
            timestamp: unsafe { os_gettime_ns() },
        };
        for source in sources.iter() {
            // SAFETY: the source is alive while it is in SOURCES (locked).
            unsafe { obs_source_output_audio(source.0, &audio) };
        }
    });
}

fn to_float(samples: &[i16], out: &mut Vec<f32>) {
    out.clear();
    out.extend(samples.iter().map(|&s| f32::from(s) / 32768.0));
}

