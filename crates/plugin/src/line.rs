// SPDX-License-Identifier: GPL-2.0-or-later
//! The studio line as the plugin runs it: settings, the core runtime with
//! PJSIP, and the return tap. Applying new settings restarts all three.
//! All functions run on OBS's UI thread.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::Mutex;

use softphone_core::Event;
use softphone_core::runtime::Runtime;
use softphone_sip::{AudioIo, OsKeyring, PjsipEngine, Settings};

use crate::{caller_source, dock, obs, return_tap};

struct State {
    path: PathBuf,
    settings: Settings,
    runtime: Option<Runtime>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

unsafe extern "C" {
    fn sp_settings_open(parent: *mut c_void, json: *const c_char, on_save: extern "C" fn(*const c_char));
}

/// At load: read (or create) the settings file and start the line.
pub fn init(path: PathBuf) {
    dock::init_chime(&path.with_file_name("chime.wav"));
    let settings = match Settings::load(&path, &OsKeyring) {
        Ok((s, notes)) => {
            notes.iter().for_each(|n| obs::log(obs::LOG_INFO, &format!("settings: {n}")));
            s
        }
        Err(e) => {
            obs::log(obs::LOG_ERROR, &format!("settings: {e}"));
            dock::show_setup(&format!("Can't read settings: {e}"));
            Settings::default()
        }
    };
    let mut state = State { path, settings, runtime: None };
    start(&mut state);
    *STATE.lock().unwrap() = Some(state);
}

/// At unload: hang up, unregister, stop PJSIP and the tap (DESIGN.md §4.2).
pub fn shutdown() {
    if let Some(mut state) = STATE.lock().unwrap().take() {
        stop(&mut state);
    }
}

fn start(state: &mut State) {
    let s = &state.settings;
    dock::set_chime(s.ring_chime);
    if !s.is_configured() {
        obs::log(obs::LOG_WARNING, "not configured; open Tools → SIP Call-In…");
        dock::show_setup("Enter your SIP server, username and password to receive calls.");
        return;
    }

    return_tap::start(s.return_mix_index());
    let ui = dock::DockUi { extension: s.username.clone() };
    let sip_cfg = s.sip_config();
    let verbose = s.log_level >= 4;
    let started = Runtime::start(s.line_config(), ui, move |sink| {
        PjsipEngine::start(
            &sip_cfg,
            sink,
            Box::new(move |level, msg| log_pjsip(level, msg, verbose)),
            AudioIo { from_caller: Box::new(caller_source::push), to_caller: Box::new(return_tap::fill) },
        )
    });
    match started {
        Ok(runtime) => {
            let sink = runtime.sink();
            dock::connect(Some(sink.clone()));
            if s.enabled {
                sink.send(Event::Enable);
            }
            state.runtime = Some(runtime);
        }
        Err(e) => {
            return_tap::stop();
            obs::log(obs::LOG_ERROR, &format!("SIP start failed: {e}"));
            dock::show_setup(&format!("SIP start failed: {e}"));
        }
    }
}

fn stop(state: &mut State) {
    dock::connect(None);
    if let Some(runtime) = state.runtime.take() {
        runtime.shutdown();
    }
    return_tap::stop();
}

/// Tools menu or the dock's Settings button.
pub fn open_settings() {
    let json = {
        let guard = STATE.lock().unwrap();
        let settings = guard.as_ref().map(|s| s.settings.clone()).unwrap_or_default();
        serde_json::to_string(&settings).expect("settings serialize")
    };
    let json = CString::new(json).expect("no NUL in JSON");
    // SAFETY: UI thread; the dialog copies the JSON before returning.
    unsafe { sp_settings_open(obs::main_window(), json.as_ptr(), on_settings_saved) };
}

extern "C" fn on_settings_saved(json: *const c_char) {
    // SAFETY: the dialog passes a NUL-terminated string for this call.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };

    let merged = match merge(&state.settings, &json) {
        Ok(s) => s,
        Err(e) => {
            obs::log(obs::LOG_ERROR, &format!("settings dialog: {e}"));
            return;
        }
    };
    match merged.save(&state.path, &OsKeyring, Some(&state.settings)) {
        Ok(Some(note)) => obs::log(obs::LOG_WARNING, &format!("settings: {note}")),
        Ok(None) => {}
        Err(e) => obs::log(obs::LOG_ERROR, &format!("saving settings: {e}")),
    }
    obs::log(obs::LOG_INFO, "settings saved; restarting the line");
    stop(state);
    state.settings = merged;
    start(state);
}

/// The dock's Auto-answer checkbox: remembered without restarting the line.
pub fn remember_auto_answer(on: bool) {
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };
    if state.settings.auto_answer != on {
        state.settings.auto_answer = on;
        if let Err(e) = state.settings.save(&state.path, &OsKeyring, None) {
            obs::log(obs::LOG_ERROR, &format!("saving settings: {e}"));
        }
    }
}

/// Overlays the dialog's fields on the current settings, keeping the ones
/// the dialog doesn't show (log level, CA file, ...).
fn merge(current: &Settings, dialog_json: &str) -> Result<Settings, String> {
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let changes: serde_json::Value = serde_json::from_str(dialog_json).map_err(|e| e.to_string())?;
    if let (Some(base), Some(changes)) = (value.as_object_mut(), changes.as_object()) {
        for (k, v) in changes {
            base.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// OBS drops LOG_DEBUG unless run with --verbose, so when the settings ask
/// for pjsip's detail (log_level ≥ 4) it is logged at LOG_INFO.
fn log_pjsip(level: i32, msg: &str, verbose: bool) {
    let level = match level {
        ..=1 => obs::LOG_ERROR,
        2 => obs::LOG_WARNING,
        3 => obs::LOG_INFO,
        _ if verbose => obs::LOG_INFO,
        _ => obs::LOG_DEBUG,
    };
    obs::log(level, &format!("pjsip: {msg}"));
}
