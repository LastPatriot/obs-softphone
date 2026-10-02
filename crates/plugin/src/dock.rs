// SPDX-License-Identifier: GPL-2.0-or-later
//! Rust side of the Qt dock (dock/sp_dock.cpp): shows the line state and
//! turns button presses into core events.

use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use softphone_core::runtime::EventSink;
use softphone_core::{Event, LineView, Status, Ui};

use crate::obs;

const ACT_ANSWER: c_int = 1;
const ACT_END: c_int = 2;
const ACT_RETRY: c_int = 3;
const ACT_TAKE_BACK: c_int = 4;
const ACT_AUTO_ON: c_int = 5;
const ACT_AUTO_OFF: c_int = 6;
const ACT_FIX_MIXMINUS: c_int = 7;
const ACT_MUTE_TOGGLE: c_int = 8;
const ACT_SETTINGS: c_int = 9;
const ACT_ADD_TO_SCENE: c_int = 10;

/// The button next to a dock warning.
#[derive(Clone, Copy)]
pub enum WarnButton {
    None = 0,
    Fix = 1,
    AddToScene = 2,
}

unsafe extern "C" {
    fn sp_dock_create(on_action: extern "C" fn(c_int), footer: *const c_char) -> *mut c_void;
    fn sp_dock_update(status: c_int, headline: *const c_char, caller: *const c_char, message: *const c_char, auto_answer: c_int);
    fn sp_dock_shutdown();
    fn sp_dock_set_warning(text: *const c_char, button: c_int);
    fn sp_dock_set_muted(muted: c_int);
    fn sp_dock_set_chime(on: c_int);
    fn sp_chime_init(path: *const c_char) -> c_int;
}

/// Where dock clicks go. Empty when the line isn't running.
static SINK: Mutex<Option<EventSink>> = Mutex::new(None);
/// Cleared at unload before the line thread stops, so no update is queued
/// to the UI after that.
static LIVE: AtomicBool = AtomicBool::new(false);

/// UI thread. Returns the widget for `obs_frontend_add_dock_by_id`.
pub fn create() -> *mut c_void {
    let footer = CString::new(format!("obs-softphone {}", env!("CARGO_PKG_VERSION"))).unwrap();
    LIVE.store(true, Ordering::SeqCst);
    unsafe { sp_dock_create(on_action, footer.as_ptr()) }
}

/// Where dock clicks go; `None` while the line is stopped or restarting.
pub fn connect(sink: Option<EventSink>) {
    *SINK.lock().unwrap() = sink;
    crate::checks::reset();
}

/// UI thread, at unload: stop updates, then (after the line thread has
/// stopped) drop anything still queued.
pub fn begin_shutdown() {
    LIVE.store(false, Ordering::SeqCst);
    SINK.lock().unwrap().take();
}

pub fn finish_shutdown() {
    unsafe { sp_dock_shutdown() }
}

/// Any thread. `None` clears it.
pub fn set_warning(text: Option<&str>, button: WarnButton) {
    if !LIVE.load(Ordering::SeqCst) {
        return;
    }
    let c = CString::new(text.unwrap_or("").replace('\0', " ")).unwrap();
    let button = if text.is_some() { button } else { WarnButton::None };
    unsafe { sp_dock_set_warning(c.as_ptr(), button as c_int) };
}

/// UI thread: prepares the chime sound (written next to config.json).
pub fn init_chime(path: &std::path::Path) {
    let c = CString::new(path.to_string_lossy().into_owned()).unwrap_or_default();
    if unsafe { sp_chime_init(c.as_ptr()) } != 0 {
        obs::log(obs::LOG_WARNING, "ring chime unavailable");
    }
}

/// Whether ringing plays the chime (from the settings).
pub fn set_chime(on: bool) {
    if LIVE.load(Ordering::SeqCst) {
        unsafe { sp_dock_set_chime(c_int::from(on)) };
    }
}

/// Any thread: the Mute caller button's state.
pub fn set_muted(muted: bool) {
    if LIVE.load(Ordering::SeqCst) {
        unsafe { sp_dock_set_muted(c_int::from(muted)) };
    }
}

/// The line can't start (not configured, bad settings): offer the settings.
pub fn show_setup(message: &str) {
    crate::checks::IN_CALL.store(false, Ordering::Relaxed);
    if LIVE.load(Ordering::SeqCst) {
        let (h, m) = (CString::new("Not set up").unwrap(), CString::new(message.replace('\0', " ")).unwrap());
        unsafe { sp_dock_update(ST_SETUP, h.as_ptr(), c"".as_ptr(), m.as_ptr(), 1) };
    }
}

const ST_SETUP: c_int = 7;

extern "C" fn on_action(action: c_int) {
    let event = match action {
        ACT_ANSWER => Event::Answer,
        ACT_END => Event::EndCall,
        ACT_RETRY => Event::Retry,
        ACT_TAKE_BACK => Event::TakeBack,
        ACT_AUTO_ON | ACT_AUTO_OFF => {
            let on = action == ACT_AUTO_ON;
            crate::line::remember_auto_answer(on);
            Event::SetAutoAnswer(on)
        }
        // The rest act on OBS or the plugin directly (UI thread), not the core.
        ACT_FIX_MIXMINUS => {
            crate::caller_source::remove_from_track(crate::caller_source::return_mix());
            return;
        }
        ACT_MUTE_TOGGLE => {
            crate::caller_source::toggle_mute();
            return;
        }
        ACT_SETTINGS => {
            crate::line::open_settings();
            return;
        }
        ACT_ADD_TO_SCENE => {
            crate::caller_source::add_to_current_scene();
            return;
        }
        _ => return,
    };
    if let Some(sink) = SINK.lock().unwrap().as_ref() {
        sink.send(event);
    }
}

fn update(status: Status, headline: &str, caller: &str, message: &str, auto_answer: bool) {
    if !LIVE.load(Ordering::SeqCst) {
        return;
    }
    let c = |s: &str| CString::new(s.replace('\0', " ")).unwrap();
    let (h, c_, m) = (c(headline), c(caller), c(message));
    let code = match status {
        Status::Disabled => 0,
        Status::Connecting => 1,
        Status::Ready => 2,
        Status::Ringing => 3,
        Status::OnAir => 4,
        Status::Error => 5,
        Status::Replaced => 6,
    };
    unsafe { sp_dock_update(code, h.as_ptr(), c_.as_ptr(), m.as_ptr(), c_int::from(auto_answer)) };
}

/// The core's [`Ui`]: updates the dock and logs each change.
pub struct DockUi {
    pub extension: String,
}

impl Ui for DockUi {
    fn show(&mut self, v: &LineView) {
        crate::checks::IN_CALL.store(matches!(v.status, Status::Ringing | Status::OnAir), Ordering::Relaxed);
        let ext = &self.extension;
        let headline = match v.status {
            Status::Disabled => "Off".to_string(),
            Status::Connecting => "Connecting…".to_string(),
            Status::Ready | Status::Ringing | Status::OnAir => format!("● Ready · {ext}"),
            Status::Error => "Can't connect".to_string(),
            Status::Replaced => format!("⚠ Another device took {ext}"),
        };
        let caller = v.caller.as_deref().unwrap_or("");
        let message = v.message.as_deref().unwrap_or("");

        let mut log = format!("line: {:?}", v.status);
        if !v.auto_answer {
            log += " (auto-answer off)";
        }
        if !caller.is_empty() {
            log += &format!(" · {caller}");
        }
        if !message.is_empty() {
            log += &format!(" · {message}");
        }
        let level = if v.status == Status::Error { obs::LOG_WARNING } else { obs::LOG_INFO };
        obs::log(level, &log);

        update(v.status, &headline, caller, message, v.auto_answer);
    }
}
